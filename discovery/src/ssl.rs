use std::time::Duration;

use anyhow::Result;
use num_bigint::BigUint as NumBigUint;
use num_integer::Integer;
use num_traits::{ToPrimitive, identities::Zero};
use rcgen::{
    CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    KeyUsagePurpose, SanType,
};
use rsa::{
    RsaPrivateKey, RsaPublicKey,
    pkcs8::{EncodePrivateKey, EncodePublicKey},
};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;

const BASE85: [char; 85] = [
    'ᚠ', 'ᚡ', 'ᚢ', 'ᚣ', 'ᚤ', 'ᚥ', 'ᚦ', 'ᚧ', 'ᚨ', 'ᚩ', 'ᚪ', 'ᚫ', 'ᚬ', 'ᚭ', 'ᚮ', 'ᚯ', 'ᚰ', 'ᚱ', 'ᚲ',
    'ᚳ', 'ᚴ', 'ᚵ', 'ᚶ', 'ᚷ', 'ᚸ', 'ᚹ', 'ᚺ', 'ᚻ', 'ᚼ', 'ᚽ', 'ᚾ', 'ᚿ', 'ᛀ', 'ᛁ', 'ᛂ', 'ᛃ', 'ᛄ', 'ᛅ',
    'ᛆ', 'ᛇ', 'ᛈ', 'ᛉ', 'ᛊ', 'ᛋ', 'ᛌ', 'ᛍ', 'ᛎ', 'ᛏ', 'ᛐ', 'ᛑ', 'ᛒ', 'ᛓ', 'ᛔ', 'ᛕ', 'ᛖ', 'ᛗ', 'ᛘ',
    'ᛙ', 'ᛚ', 'ᛛ', 'ᛜ', 'ᛝ', 'ᛞ', 'ᛟ', 'ᛠ', 'ᛡ', 'ᛢ', 'ᛣ', 'ᛤ', 'ᛥ', 'ᛦ', 'ᛨ', 'ᛩ', 'ᛪ', 'ᛮ', 'ᛯ',
    'ᛰ', 'ᛱ', 'ᛲ', 'ᛳ', 'ᛴ', 'ᛵ', 'ᛶ', 'ᛷ', 'ᛸ',
];

pub struct CertificateResult {
    pub private_key: String,
    pub public_key: String,
    pub certificate: String,
    pub public_key_fingerprint: String,
}

pub fn generate_self_signed_cert(
    common_name: &str,
    organization: &str,
    country: &str,
    validity_days: u32,
) -> Result<CertificateResult> {
    let mut rng = rand::thread_rng();
    let bits = 2048;

    let private_key = RsaPrivateKey::new(&mut rng, bits)?;
    let public_key = RsaPublicKey::from(&private_key);

    let mut params = CertificateParams::default();
    params.distinguished_name = create_distinguished_name(common_name, organization, country);
    params.is_ca = IsCa::NoCa;
    params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyEncipherment,
    ];
    params.extended_key_usages = vec![
        ExtendedKeyUsagePurpose::ServerAuth,
        ExtendedKeyUsagePurpose::ClientAuth,
    ];
    params.serial_number = Some(rand::random::<u64>().to_be_bytes().to_vec().into());

    // Create Ia5String using from() instead of new()
    params.subject_alt_names = vec![SanType::DnsName(common_name.to_string().try_into()?)];

    let now = OffsetDateTime::now_utc();
    params.not_before = now;
    params.not_after = now + Duration::from_secs(validity_days as u64 * 86400);

    let pkcs8_der = private_key.to_pkcs8_der()?;
    let key_pair = KeyPair::try_from(pkcs8_der.as_bytes())?;

    let cert = params.self_signed(&key_pair)?;

    let private_key_pem = private_key.to_pkcs8_pem(rsa::pkcs8::LineEnding::LF)?;
    let public_key_pem = public_key.to_public_key_pem(rsa::pkcs8::LineEnding::LF)?;
    let certificate_pem = cert.pem();

    let pem_entry = pem::parse(public_key_pem.as_bytes())?;
    let public_key_der = pem_entry.contents();
    let fingerprint = calculate_base85_fingerprint(public_key_der)?;

    Ok(CertificateResult {
        private_key: private_key_pem.to_string(),
        public_key: public_key_pem,
        certificate: certificate_pem,
        public_key_fingerprint: fingerprint,
    })
}

pub fn create_distinguished_name(
    common_name: &str,
    organization: &str,
    country: &str,
) -> DistinguishedName {
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, common_name);
    dn.push(DnType::OrganizationName, organization);
    dn.push(DnType::CountryName, country);
    dn
}

pub fn calculate_base85_fingerprint(data: &[u8]) -> Result<String> {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let hash = hasher.finalize();

    let num = NumBigUint::from_bytes_be(&hash);
    let base = NumBigUint::from(85u32);
    let mut n = num;
    let mut chars = Vec::new();

    if n.is_zero() {
        return Ok(BASE85[0].to_string());
    }

    while !n.is_zero() {
        let (quotient, remainder) = n.div_rem(&base);
        chars.push(BASE85[remainder.to_usize().unwrap()]);
        n = quotient;
    }
    chars.reverse();

    let min_length = (hash.len() * 8) as f64 / 6.409;
    while chars.len() < min_length.ceil() as usize {
        chars.insert(0, BASE85[0]);
    }

    Ok(chars.into_iter().collect())
}
