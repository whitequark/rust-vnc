use std::cmp::min;
use octavo::crypto::asymmetric::dh::{DHParameters, DHPublicKey};
use num_bigint::BigUint;
use crypto::aessafe::AesSafe128Encryptor;
use crypto::symmetriccipher::BlockEncryptor;
use ::protocol;
use security::md5::md5;

// http://cafbit.com/entry/apple_remote_desktop_quirks
pub fn apple_auth(username: &str, password: &str,
                  handshake: &protocol::AppleAuthHandshake) -> protocol::AppleAuthResponse {
    let param = DHParameters::new(&handshake.prime, handshake.generator as u64);
    let priv_key = param.private_key();
    let pub_key = priv_key.public_key();
    let secret =
        md5(
            &priv_key.exchange(&DHPublicKey::new(
                BigUint::from_bytes_be(&handshake.peer_key)
            )).to_bytes_be()
        );

    let mut credentials = [0u8; 128];
    let ul = min(64, username.len());
    credentials[0..ul].copy_from_slice(&username.as_bytes()[0..ul]);
    let pl = min(64, password.len());
    credentials[64..(64 + pl)].copy_from_slice(&password.as_bytes()[0..pl]);

    let mut ciphertext = [0u8; 128];
    let aes = AesSafe128Encryptor::new(&secret);
    for i in 0..(credentials.len() / aes.block_size()) {
        let start = i * aes.block_size();
        let end = (i + 1) * aes.block_size();
        let input = &credentials[start..end];
        aes.encrypt_block(input, &mut ciphertext[start..end]);
    }

    protocol::AppleAuthResponse {
        ciphertext: ciphertext,
        pub_key: pub_key.key().to_bytes_be(),
    }
}
