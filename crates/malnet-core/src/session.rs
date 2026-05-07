use rand::{rng, Rng};
use sha2::{Sha256, Digest};
use x25519_dalek::{PublicKey, EphemeralSecret, SharedSecret};
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Nonce,
};


/// Sessions are meant to be attached to every client and every server
/// They primarily act as a layer of authentication to allow clients and servers to encrypt/decrypt packets
///
/// `@ephemeral_secret`: The secret key generated on the computer this session was created from
/// `@public_key`: This key gets sent to the server to be used for computing the rest of the encryption logic
///
/// All sessions are created on the client side first and then requested to the server for processing
///
/// Session Logic & Usage:
/// 1. The client creates a new Session when they wish to connect to any server
/// 2. The client sends their first packet which must contain their public key
/// 3. The server reads the plaintext foreign public key and creates their own ephemeral secret, public key & shared secret & master key
/// 4. The server sends the client their public key
/// 5. The client reads the plaintext foreign public key and creates their shared secret
/// 6. The client computes for the master key using the shared secret
///
/// 7. All packets can now use the master key for encryption and decryption
///
///
pub struct Session {
    pub ephemeral_secret: Option<EphemeralSecret>,
    pub public_key: Option<PublicKey>,
    pub shared_secret: Option<SharedSecret>,
    pub master_key: [u8; 32],
}


/// Fairly simple usage example:
/// Sender (client or server) – no setup beyond the symmetric key
/// let encrypted = encrypt_packet(&sym_key, b"player_jump");
/// socket.send_to(&encrypted, addr).unwrap();
///
/// Receiver – just the key and the raw received bytes
/// let received_bytes = ... // from socket
/// if let Some(plaintext) = decrypt_packet(&sym_key, &received_bytes) {
///     // plaintext is the original message, tamper-proof and confidential
/// }

impl Session {

    /// Generates a new Session which contains the ephemeral secret and relative public key
    ///
    /// NOTE: The shared_secret AND master_key will NOT exist to begin with and will only be populated once the full Session process finishes
    pub async fn new() -> Self {
        let _ephemeral_secret = Self::generate_ephemeral_secret().await;
        let _public_key = Self::public_key_from_ephemeral_secret(&_ephemeral_secret).await;

        Self {
            ephemeral_secret: Some(_ephemeral_secret),
            public_key: Some(_public_key),
            shared_secret: None,
            master_key: [0u8; 32]
        }
    }

    // TODO What is an Ephemeral secret and what is happening here?
    pub async fn generate_ephemeral_secret() -> EphemeralSecret {
        let mut rng = rand::rng();          // ThreadRng – cryptographically secure
        EphemeralSecret::random_from_rng(&mut rng)
    }


    /// This does the X25519 scalar multiplication: public = secret * base point.
    pub async fn public_key_from_ephemeral_secret(secret: &EphemeralSecret) -> PublicKey {
        PublicKey::from(secret)
    }

    /// `secret` is consumed to ensure perfect forward secrecy.
    /// Returns the raw 32‑byte shared secret, which never touches the network.
    pub async fn diffie_hellman(
        ephemeral_secret: EphemeralSecret,
        foreign_public_key: &PublicKey,
    ) -> [u8; 32] {
        // diffie_hellman() consumes any ephemeral secret passed into it
        let shared_secret: SharedSecret = ephemeral_secret.diffie_hellman(foreign_public_key);
        *shared_secret.as_bytes()
    }


    /// Hashes the 32‑byte shared secret into a uniformly random 32‑byte key.
    pub async fn derive_master_key(shared_secret: &[u8; 32]) -> [u8; 32] {
        let mut hasher: Sha256 = Sha256::new();
        hasher.update(shared_secret);
        hasher.finalize().into()
    }


    /// Encrypt plaintext using only the master key.
    /// Returns a self-contained packet: [12-byte random nonce][ciphertext+tag].
    pub async fn encrypt_packet(key: &[u8; 32], plaintext: &[u8]) -> Vec<u8> {
        let cipher = ChaCha20Poly1305::new_from_slice(key).unwrap();

        // Random nonce – no counters, no coordination needed
        let mut nonce = [0u8; 12];
        rng().fill_bytes(&mut nonce);

        let mut ciphertext = cipher.encrypt(Nonce::from_slice(&nonce), plaintext).unwrap();

        // Glue nonce + encrypted data together
        let mut packet = Vec::with_capacity(12 + ciphertext.len());
        packet.extend_from_slice(&nonce);
        packet.append(&mut ciphertext);
        packet
    }

    /// Decrypt a packet that was created by `encrypt_packet`.
    /// Only needs the symmetric key. Returns `Some(plaintext)` if valid, `None` if forged/corrupt.
    pub async fn decrypt_packet(key: &[u8; 32], packet: &[u8]) -> Option<Vec<u8>> {
        if packet.len() < 28 { return None; } // 12 Nonce + 16 tag minimum

        let (nonce, ciphertext) = packet.split_at(12);
        let cipher = ChaCha20Poly1305::new_from_slice(key).unwrap();
        cipher.decrypt(Nonce::from_slice(nonce), ciphertext).ok()
    }


}