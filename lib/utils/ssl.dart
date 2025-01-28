import 'dart:math';
import 'dart:convert';
import 'dart:typed_data';

import 'package:asn1lib/asn1lib.dart';
import 'package:pointycastle/export.dart';

class CertificateResult {
  final String privateKey;
  final String publicKey;
  final String certificate;
  final String publicKeyFingerprint;

  CertificateResult({
    required this.privateKey,
    required this.publicKey,
    required this.certificate,
    required this.publicKeyFingerprint,
  });
}

Future<CertificateResult> generateSelfSignedCertificate({
  required String commonName,
  required String organization,
  required String country,
  int validityDays = 365,
}) async {
  // Generate RSA key pair (2048-bit)
  final keyPair = await _generateKeyPair();
  final publicKey = keyPair.publicKey;
  final privateKey = keyPair.privateKey;

  // Create X.509 certificate subject (DN - Distinguished Name)
  final subject = _createDistinguishedName(
    commonName: commonName,
    organization: organization,
    country: country,
  );

  // Create X.509 certificate structure
  final cert = _createCertificate(
    subject: subject,
    publicKey: publicKey,
    privateKey: privateKey,
    validityDays: validityDays,
  );

  // Convert all components to PEM format
  final privateKeyPem = _encodePrivateKeyToPem(privateKey, publicKey.exponent!);
  final publicKeyPem = _encodePublicKeyToPem(publicKey);
  final certificatePem = _encodeCertificateToPem(cert);

  final publicKeyFingerprint = getPublicKeyFingerprintBase85(publicKey);

  return CertificateResult(
    privateKey: privateKeyPem,
    publicKey: publicKeyPem,
    certificate: certificatePem,
    publicKeyFingerprint: publicKeyFingerprint,
  );
}

/// Generates RSA key pair using Fortuna PRNG and RSA key generator
Future<AsymmetricKeyPair<RSAPublicKey, RSAPrivateKey>>
    _generateKeyPair() async {
  // Initialize secure random number generator
  final secureRandom = FortunaRandom();
  final seedSource = Random.secure();
  final seeds = List<int>.generate(32, (_) => seedSource.nextInt(256));
  secureRandom.seed(KeyParameter(Uint8List.fromList(seeds)));

  // Configure RSA key generator (2048-bit, public exponent 65537)
  final keyGen = RSAKeyGenerator()
    ..init(ParametersWithRandom(
      RSAKeyGeneratorParameters(BigInt.parse('65537'), 2048, 64),
      secureRandom,
    ));

  // Generate and return key pair
  final pair = keyGen.generateKeyPair();
  return AsymmetricKeyPair<RSAPublicKey, RSAPrivateKey>(
    pair.publicKey as RSAPublicKey,
    pair.privateKey as RSAPrivateKey,
  );
}

/// Creates ASN.1 Distinguished Name (DN) structure
ASN1Sequence _createDistinguishedName({
  required String commonName,
  required String organization,
  required String country,
}) {
  final sequence = ASN1Sequence();

  // Add Common Name (CN) component
  sequence.add(ASN1Set()
    ..add(ASN1Sequence()
      ..add(ASN1ObjectIdentifier([2, 5, 4, 3])) // OID for CN (CommonName)
      ..add(ASN1PrintableString(commonName))));

  // Add Organization (O) component
  sequence.add(ASN1Set()
    ..add(ASN1Sequence()
      ..add(ASN1ObjectIdentifier([2, 5, 4, 10])) // OID for O (Organization)
      ..add(ASN1PrintableString(organization))));

  // Add Country (C) component
  sequence.add(ASN1Set()
    ..add(ASN1Sequence()
      ..add(ASN1ObjectIdentifier([2, 5, 4, 6])) // OID for C (CountryName)
      ..add(ASN1PrintableString(country))));

  return sequence;
}

ASN1Object _createTime(DateTime date) {
  final utcDate = date.toUtc();
  return utcDate.year >= 2050
      ? ASN1GeneralizedTime(utcDate)
      : ASN1UtcTime(utcDate);
}

/// Helper function to encode ASN.1 length
List<int> _encodeLength(int length) {
  if (length < 128) {
    return [length];
  }

  final bytes = <int>[];
  var len = length;
  while (len > 0) {
    bytes.insert(0, len & 0xff);
    len >>= 8;
  }

  return [0x80 | bytes.length] + bytes;
}

/// Constructs X.509 certificate structure
ASN1Sequence _createCertificate({
  required ASN1Sequence subject,
  required RSAPublicKey publicKey,
  required RSAPrivateKey privateKey,
  required int validityDays,
}) {
  final now = DateTime.now();
  final notBefore = _createTime(now);
  final notAfter = _createTime(now.add(Duration(days: validityDays)));

  // Create version as explicit tagged object manually
  final versionBytes = ASN1Integer(BigInt.from(2)).encodedBytes;
  final List<int> versionTagBytesList =
      [0xA0] + _encodeLength(versionBytes.length) + versionBytes;
  final versionTagBytes = Uint8List.fromList(versionTagBytesList);
  final version = ASN1Object.fromBytes(versionTagBytes);

  final random = Random.secure();
  final serialBytes = List<int>.generate(19, (_) => random.nextInt(256));
  serialBytes.insert(0, random.nextInt(127));
  final serialNumber = ASN1Integer(BigInt.parse(
      serialBytes.map((e) => e.toRadixString(16).padLeft(2, '0')).join(),
      radix: 16));

  final basicConstraintsExt = ASN1Sequence()
    ..add(ASN1ObjectIdentifier([2, 5, 29, 19])) // basicConstraints OID
    ..add(ASN1Boolean(true)) // critical
    ..add(ASN1OctetString(Uint8List.fromList(
        (ASN1Sequence()..add(ASN1Boolean(true))).encodedBytes)));

  final extensions = ASN1Sequence();
  extensions.add(basicConstraintsExt);

  final extensionsBytes = extensions.encodedBytes;
  final List<int> extensionsTagBytesList =
      [0xA3] + _encodeLength(extensionsBytes.length) + extensionsBytes;
  final extensionsTagBytes = Uint8List.fromList(extensionsTagBytesList);
  final taggedExtensions = ASN1Object.fromBytes(extensionsTagBytes);

  // Create TBSCertificate structure (To Be Signed Certificate)
  final tbsCertificate = ASN1Sequence()
    ..add(version) // isExplicit = true
    ..add(serialNumber) // Serial number
    ..add(ASN1Sequence() // Signature algorithm identifier
      ..add(ASN1ObjectIdentifier(
          [1, 2, 840, 113549, 1, 1, 11])) // SHA-256 with RSA Encryption OID
      ..add(ASN1Null())) // Parameters (null for RSA)
    ..add(subject) // Issuer (same as subject for self-signed)
    ..add(ASN1Sequence() // Validity period
      ..add(notBefore)
      ..add(notAfter))
    ..add(subject) // Subject
    ..add(_createSubjectPublicKeyInfo(publicKey))
    ..add(taggedExtensions); // Subject public key info

  // Sign the TBSCertificate using private key
  final signer = RSASigner(SHA256Digest(), '06092a864886f70d01010b');
  final privParams = PrivateKeyParameter<RSAPrivateKey>(privateKey);
  signer.init(true, privParams);

  final signature = signer.generateSignature(tbsCertificate.encodedBytes);
  final signatureBytes = signature.bytes;

  // Assemble final certificate structure
  final certificate = ASN1Sequence()
    ..add(tbsCertificate)
    ..add(ASN1Sequence() // Signature algorithm identifier
      ..add(ASN1ObjectIdentifier([1, 2, 840, 113549, 1, 1, 11]))
      ..add(ASN1Null()))
    ..add(ASN1BitString(signatureBytes)); // Digital signature

  return certificate;
}

/// Creates SubjectPublicKeyInfo structure (ASN.1 format)
ASN1Sequence _createSubjectPublicKeyInfo(RSAPublicKey publicKey) {
  // Algorithm identifier for RSA encryption
  final algorithm = ASN1Sequence()
    ..add(ASN1ObjectIdentifier([1, 2, 840, 113549, 1, 1, 1])) // RSA OID
    ..add(ASN1Null()); // Parameters (null for RSA)

  // RSA public key components
  final subjectPublicKey = ASN1Sequence()
    ..add(ASN1Integer(publicKey.modulus!)) // n - modulus
    ..add(ASN1Integer(publicKey.exponent!)); // e - public exponent

  return ASN1Sequence()
    ..add(algorithm)
    ..add(ASN1BitString(subjectPublicKey.encodedBytes));
}

/// Encodes private key to PKCS#8 PEM format
String _encodePrivateKeyToPem(RSAPrivateKey privateKey, BigInt publicExponent) {
  // First create PKCS#1 RSAPrivateKey structure
  final pkcs1PrivateKey = ASN1Sequence()
    ..add(ASN1Integer(BigInt.from(0))) // Version
    ..add(ASN1Integer(privateKey.modulus!))
    ..add(ASN1Integer(publicExponent))
    ..add(ASN1Integer(privateKey.privateExponent!))
    ..add(ASN1Integer(privateKey.p!))
    ..add(ASN1Integer(privateKey.q!))
    ..add(
        ASN1Integer(privateKey.privateExponent! % (privateKey.p! - BigInt.one)))
    ..add(
        ASN1Integer(privateKey.privateExponent! % (privateKey.q! - BigInt.one)))
    ..add(ASN1Integer(privateKey.q!.modInverse(privateKey.p!)));

  // Create algorithm identifier for RSA
  final algorithmSeq = ASN1Sequence()
    ..add(ASN1ObjectIdentifier([1, 2, 840, 113549, 1, 1, 1])) // rsaEncryption
    ..add(ASN1Null());

  // Wrap PKCS#1 in PKCS#8 structure
  final pkcs8PrivateKey = ASN1Sequence()
    ..add(ASN1Integer(BigInt.from(0))) // Version (v1)
    ..add(algorithmSeq)
    ..add(ASN1OctetString(pkcs1PrivateKey.encodedBytes));

  final bytes = pkcs8PrivateKey.encodedBytes;
  final base64 = base64Encode(bytes);
  return '''-----BEGIN PRIVATE KEY-----\n${_wrapBase64(base64)}\n-----END PRIVATE KEY-----''';
}

/// Encodes public key to X.509 SubjectPublicKeyInfo PEM format
String _encodePublicKeyToPem(RSAPublicKey publicKey) {
  final spki = _createSubjectPublicKeyInfo(publicKey);
  final bytes = spki.encodedBytes;
  final base64 = base64Encode(bytes);
  return '''-----BEGIN PUBLIC KEY-----\n${_wrapBase64(base64)}\n-----END PUBLIC KEY-----''';
}

/// Encodes certificate to X.509 PEM format
String _encodeCertificateToPem(ASN1Sequence certificate) {
  final bytes = certificate.encodedBytes;
  final base64 = base64Encode(bytes);
  return '''-----BEGIN CERTIFICATE-----\n${_wrapBase64(base64)}\n-----END CERTIFICATE-----''';
}

/// Helper to split base64 string into 64-character chunks
String _wrapBase64(String input) {
  final chunks = <String>[];
  for (var i = 0; i < input.length; i += 64) {
    chunks.add(input.substring(i, min(i + 64, input.length)));
  }
  return chunks.join('\n');
}

String getPublicKeyFingerprint(RSAPublicKey publicKey) {
  // 1. Get the DER encoding of the public key (using the existing SPKI structure)
  final spki = _createSubjectPublicKeyInfo(publicKey);
  final derBytes = spki.encodedBytes;

  // 2. Calculate the SHA-256 hash
  final hash = SHA256Digest().process(Uint8List.fromList(derBytes));

  // 3. Convert to a hexadecimal string with colon separators
  return _bytesToHexColon(hash);
}

// Helper method: Convert a byte array to a colon-separated hexadecimal string
String _bytesToHexColon(List<int> bytes) {
  return bytes
      .map((byte) => byte.toRadixString(16).padLeft(2, '0'))
      .join(':')
      .toUpperCase();
}

const _base85 = [
  'ᚠ', 'ᚡ', 'ᚢ', 'ᚣ', 'ᚤ', 'ᚥ', 'ᚦ', 'ᚧ', 'ᚨ', 'ᚩ', // 0-9
  'ᚪ', 'ᚫ', 'ᚬ', 'ᚭ', 'ᚮ', 'ᚯ', 'ᚰ', 'ᚱ', 'ᚲ', 'ᚳ', // 10-19
  'ᚴ', 'ᚵ', 'ᚶ', 'ᚷ', 'ᚸ', 'ᚹ', 'ᚺ', 'ᚻ', 'ᚼ', 'ᚽ', // 20-29
  'ᚾ', 'ᚿ', 'ᛀ', 'ᛁ', 'ᛂ', 'ᛃ', 'ᛄ', 'ᛅ', 'ᛆ', 'ᛇ', // 30-39
  'ᛈ', 'ᛉ', 'ᛊ', 'ᛋ', 'ᛌ', 'ᛍ', 'ᛎ', 'ᛏ', 'ᛐ', 'ᛑ', // 40-49
  'ᛒ', 'ᛓ', 'ᛔ', 'ᛕ', 'ᛖ', 'ᛗ', 'ᛘ', 'ᛙ', 'ᛚ', 'ᛛ', // 50-59
  'ᛜ', 'ᛝ', 'ᛞ', 'ᛟ', 'ᛠ', 'ᛡ', 'ᛢ', 'ᛣ', 'ᛤ', 'ᛥ', // 60-69
  'ᛦ', 'ᛨ', 'ᛩ', 'ᛪ', 'ᛮ', 'ᛯ', 'ᛰ', 'ᛱ', 'ᛲ', 'ᛳ', // 70-79
  'ᛴ', 'ᛵ', 'ᛶ', 'ᛷ', 'ᛸ' // 80-84
];

const log85 = 6.40939093613770175612438544029979452987609867536940130018136546020263128565;

String getPublicKeyFingerprintBase85(RSAPublicKey publicKey) {
  // 1. Get DER encoding
  final spki = _createSubjectPublicKeyInfo(publicKey);
  final derBytes = spki.encodedBytes;

  // 2. Calculate SHA-256 hash
  final hash = SHA256Digest().process(Uint8List.fromList(derBytes));

  // 3. Convert to custom base-85 string
  return _bytesToBase85(hash);
}

/// Convert a byte array to an Ogham base-85 string
String _bytesToBase85(List<int> bytes) {
  BigInt number = BigInt.zero;

  for (var byte in bytes) {
    number = (number << 8) | BigInt.from(byte);
  }

  final result = <String>[];
  final base = BigInt.from(85);

  while (number > BigInt.zero) {
    final remainder = number % base;
    result.insert(0, _base85[remainder.toInt()]);
    number = number ~/ base;
  }

  final expectedLength = (bytes.length * 8 / log85).ceil();
  while (result.length < expectedLength) {
    result.insert(0, _base85[0]);
  }

  return result.join();
}
