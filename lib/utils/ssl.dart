import 'dart:math';
import 'dart:convert';
import 'dart:typed_data';

import 'package:asn1lib/asn1lib.dart';
import 'package:pointycastle/export.dart';

class CertificateResult {
  final String privateKey;
  final String publicKey;
  final String certificate;

  CertificateResult({
    required this.privateKey,
    required this.publicKey,
    required this.certificate,
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

  return CertificateResult(
    privateKey: privateKeyPem,
    publicKey: publicKeyPem,
    certificate: certificatePem,
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

/// Encodes private key to PKCS#1 PEM format
String _encodePrivateKeyToPem(RSAPrivateKey privateKey, BigInt publicExponent) {
  // PKCS#1 private key structure
  final sequence = ASN1Sequence()
    ..add(ASN1Integer(BigInt.from(0))) // Version (PKCS#1 v1.5)
    ..add(ASN1Integer(privateKey.modulus!)) // n
    ..add(ASN1Integer(publicExponent)) // e (public exponent)
    ..add(ASN1Integer(privateKey.privateExponent!)) // d (private exponent)
    ..add(ASN1Integer(privateKey.p!)) // p (prime 1)
    ..add(ASN1Integer(privateKey.q!)) // q (prime 2)
    ..add(ASN1Integer(privateKey.privateExponent! %
        (privateKey.p! - BigInt.one))) // d mod (p-1)
    ..add(ASN1Integer(privateKey.privateExponent! %
        (privateKey.q! - BigInt.one))) // d mod (q-1)
    ..add(ASN1Integer(privateKey.q!.modInverse(privateKey.p!))); // q^-1 mod p

  final bytes = sequence.encodedBytes;
  final base64 = base64Encode(bytes);
  return '''-----BEGIN RSA PRIVATE KEY-----\n${_wrapBase64(base64)}\n-----END RSA PRIVATE KEY-----''';
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
