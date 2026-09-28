use std::{fmt, fs::File, io::Read, path::Path};

use anyhow::{Result, bail};
use serde::de::{DeserializeSeed, Error, IgnoredAny, SeqAccess, Visitor};
use solana_keypair::Keypair;
use zeroize::Zeroizing;

const MAX_KEYFILE_SIZE: usize = 4096;

pub(crate) fn read(path: &Path) -> Result<Keypair> {
    let file = File::open(path).map_err(|_| anyhow::anyhow!("Cannot open wallet keypair"))?;
    let mut encoded = Zeroizing::new(Vec::with_capacity(MAX_KEYFILE_SIZE + 1));
    file.take((MAX_KEYFILE_SIZE + 1) as u64)
        .read_to_end(&mut encoded)
        .map_err(|_| anyhow::anyhow!("Cannot read wallet keypair"))?;
    if encoded.len() > MAX_KEYFILE_SIZE {
        bail!("Keypair file is unexpectedly large");
    }
    let mut bytes = Zeroizing::new([0u8; 64]);
    let mut json = serde_json::Deserializer::from_slice(&encoded);
    KeyBytes(&mut bytes)
        .deserialize(&mut json)
        .and_then(|()| json.end())
        .map_err(|_| anyhow::anyhow!("Invalid Solana keypair JSON"))?;
    Keypair::try_from(bytes.as_slice()).map_err(|_| anyhow::anyhow!("Invalid Solana keypair JSON"))
}

// Parse directly into guarded storage, including when malformed input fails midway.
struct KeyBytes<'a>(&'a mut [u8; 64]);

impl<'de> DeserializeSeed<'de> for KeyBytes<'_> {
    type Value = ();

    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_seq(self)
    }
}

impl<'de> Visitor<'de> for KeyBytes<'_> {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a 64-byte Solana keypair array")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        for byte in self.0.iter_mut() {
            *byte = seq
                .next_element()?
                .ok_or_else(|| A::Error::custom("Missing keypair byte"))?;
        }
        if seq.next_element::<IgnoredAny>()?.is_some() {
            return Err(A::Error::custom("Extra keypair byte"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_malformed_oversized_and_mismatched_keys_without_echoing_input() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key.json");
        for input in [
            "[1,2,3]".into(),
            "[\"private-marker\"]".into(),
            " ".repeat(4097),
            format!("[{}]", vec!["0"; 64].join(",")),
        ] {
            std::fs::write(&path, &input).unwrap();
            let error = read(&path).unwrap_err().to_string();
            assert!(!error.contains("private-marker"));
            assert!(!error.contains(&input));
        }
        let mut bytes = Zeroizing::new([0u8; 64]);
        let mut parser = serde_json::Deserializer::from_str("[7,8,\"invalid\"]");
        assert!(KeyBytes(&mut bytes).deserialize(&mut parser).is_err());
        assert_eq!(&bytes[..2], &[7, 8]);
    }
}
