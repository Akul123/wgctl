use anyhow::{Context, Result};
use base64::prelude::{BASE64_STANDARD, Engine as _};
use netlink_packet_wireguard::WireguardAttribute;
use x25519_dalek::{X25519_BASEPOINT_BYTES, x25519};

pub struct WireguardKeyPair {
    private: [u8; WireguardAttribute::WG_KEY_LEN],
    public: [u8; WireguardAttribute::WG_KEY_LEN],
}

impl WireguardKeyPair {
    pub fn generate() -> Result<Self> {
        let mut private = [0_u8; WireguardAttribute::WG_KEY_LEN];

        let _ = getrandom::fill(&mut private)
            .map_err(|e| anyhow::anyhow!("Could not create private; {e}"));
        // modify random bytes using algorithm described
        // at https://cr.yp.to/ecdh.html.
        private[0] &= 248;
        private[31] &= 127;
        private[31] |= 64;

        let public = x25519(private, X25519_BASEPOINT_BYTES);

        Ok(Self { private, public })
    }

    pub fn decode_wireguard_key(
        key: String,
    ) -> anyhow::Result<[u8; WireguardAttribute::WG_KEY_LEN]> {
        // decode
        let decoded = BASE64_STANDARD
            .decode(key.trim())
            .context("given key is not BASE64")?;

        // translate to [u8; WG_KEY_LEN]
        let bytes: [u8; WireguardAttribute::WG_KEY_LEN] = decoded.as_slice().try_into()?;

        Ok(bytes)
    }

    // pub fn private_bytes(&self) -> &[u8; WireguardAttribute::WG_KEY_LEN] {
    //     &self.private
    // }

    // pub fn public_bytes(&self) -> &[u8; WireguardAttribute::WG_KEY_LEN] {
    //     &self.public
    // }

    pub fn private_base64(&self) -> String {
        BASE64_STANDARD.encode(self.private)
    }

    pub fn public_base64(&self) -> String {
        BASE64_STANDARD.encode(self.public)
    }
}
