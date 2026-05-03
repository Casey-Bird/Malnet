//! CRC32 checksum utilities for data integrity verification.

use std::io;

use crc32fast::Hasher;

/// Appends a CRC32 checksum to the encoded packet data.
/// Returns a new vector with the checksum appended.
pub fn append_checksum(data: &[u8]) -> Vec<u8> {
    let mut hasher = Hasher::new();
    hasher.update(data);
    let checksum = hasher.finalize();

    let mut result = Vec::with_capacity(data.len() + 4);
    result.extend_from_slice(data);
    result.extend_from_slice(&checksum.to_be_bytes());
    result
}

/// Appends a CRC32 checksum to the provided buffer in-place.
pub fn append_checksum_in_place(data: &mut Vec<u8>) {
    let mut hasher = Hasher::new();
    hasher.update(data);
    let checksum = hasher.finalize();
    data.extend_from_slice(&checksum.to_be_bytes());
}

/// Validates and strips the CRC32 checksum from packet data.
/// Returns the data without checksum if valid, or an error if checksum fails.
pub fn validate_and_strip_checksum(data: &[u8]) -> io::Result<&[u8]> {
    if data.len() < 4 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Data too short for checksum"));
    }

    let (payload, checksum_bytes) = data.split_at(data.len() - 4);
    let received_checksum = u32::from_be_bytes([
        checksum_bytes[0],
        checksum_bytes[1],
        checksum_bytes[2],
        checksum_bytes[3],
    ]);

    let mut hasher = Hasher::new();
    hasher.update(payload);
    let computed_checksum = hasher.finalize();

    if received_checksum != computed_checksum {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "CRC32 checksum mismatch: expected {}, got {}",
                computed_checksum, received_checksum
            ),
        ));
    }

    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_checksum_append_and_validate() {
        let data = b"Hello, world!";
        let with_checksum = append_checksum(data);
        assert_eq!(with_checksum.len(), data.len() + 4);

        let validated = validate_and_strip_checksum(&with_checksum).unwrap();
        assert_eq!(validated, data);
    }

    #[test]
    fn test_checksum_validation_fails_on_corruption() {
        let data = b"Hello, world!";
        let mut with_checksum = append_checksum(data);

        // Corrupt the checksum
        let len = with_checksum.len();
        with_checksum[len - 1] ^= 0xFF;

        assert!(validate_and_strip_checksum(&with_checksum).is_err());
    }

    #[test]
    fn test_checksum_validation_rejects_short_data() {
        let data = b"Hi";
        assert!(validate_and_strip_checksum(data).is_err());
    }

    #[test]
    fn test_checksum_with_empty_data() {
        let data = b"";
        let with_checksum = append_checksum(data);
        assert_eq!(with_checksum.len(), 4);

        let validated = validate_and_strip_checksum(&with_checksum).unwrap();
        assert_eq!(validated, data);
    }

    #[test]
    fn test_append_checksum_in_place() {
        let data = b"Test data";
        let mut buffer = data.to_vec();
        append_checksum_in_place(&mut buffer);

        assert_eq!(buffer.len(), data.len() + 4);
        let validated = validate_and_strip_checksum(&buffer).unwrap();
        assert_eq!(validated, data);
    }

    #[test]
    fn test_checksum_with_large_data() {
        // Test with 1MB of data
        let large_data = vec![0x42u8; 1024 * 1024];
        let with_checksum = append_checksum(&large_data);

        assert_eq!(with_checksum.len(), large_data.len() + 4);

        let validated = validate_and_strip_checksum(&with_checksum).unwrap();
        assert_eq!(validated, &large_data[..]);
    }

    #[test]
    fn test_checksum_detects_payload_corruption() {
        let data = b"Important data that must not be corrupted";
        let mut with_checksum = append_checksum(data);

        // Corrupt a byte in the middle of the payload
        with_checksum[20] ^= 0x01;

        let result = validate_and_strip_checksum(&with_checksum);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("CRC32 checksum mismatch"));
    }

    #[test]
    fn test_checksum_detects_checksum_corruption() {
        let data = b"Test data";
        let mut with_checksum = append_checksum(data);

        // Corrupt the checksum bytes themselves
        let len = with_checksum.len();
        with_checksum[len - 2] ^= 0x01;

        let result = validate_and_strip_checksum(&with_checksum);
        assert!(result.is_err());
    }

    #[test]
    fn test_checksum_consistency() {
        // Same data should produce same checksum
        let data = b"Consistent data";
        let checksum1 = append_checksum(data);
        let checksum2 = append_checksum(data);

        assert_eq!(checksum1, checksum2);
    }

    #[test]
    fn test_checksum_different_data_different_checksum() {
        // Different data should produce different checksums
        let data1 = b"First message";
        let data2 = b"Second message";

        let checksum1 = append_checksum(data1);
        let checksum2 = append_checksum(data2);

        // The checksums (last 4 bytes) should be different
        let cs1 = &checksum1[checksum1.len() - 4..];
        let cs2 = &checksum2[checksum2.len() - 4..];
        assert_ne!(cs1, cs2);
    }

    #[test]
    fn test_checksum_binary_data() {
        // Test with binary data (not just text)
        let binary_data = vec![0x00, 0xFF, 0xAA, 0x55, 0x12, 0x34, 0x56, 0x78];
        let with_checksum = append_checksum(&binary_data);

        let validated = validate_and_strip_checksum(&with_checksum).unwrap();
        assert_eq!(validated, &binary_data[..]);
    }

    #[test]
    fn test_checksum_single_byte_data() {
        let data = b"A";
        let with_checksum = append_checksum(data);

        assert_eq!(with_checksum.len(), 5); // 1 byte data + 4 bytes checksum

        let validated = validate_and_strip_checksum(&with_checksum).unwrap();
        assert_eq!(validated, data);
    }

    #[test]
    fn test_in_place_vs_allocating() {
        // Verify both methods produce identical results
        let data = b"Test data for comparison";

        let allocated = append_checksum(data);

        let mut in_place = data.to_vec();
        append_checksum_in_place(&mut in_place);

        assert_eq!(allocated, in_place);
    }

    #[test]
    fn test_checksum_validation_error_message() {
        let data = b"Test";
        let mut with_checksum = append_checksum(data);

        // Corrupt it
        with_checksum[0] ^= 0xFF;

        let result = validate_and_strip_checksum(&with_checksum);
        assert!(result.is_err());

        let err = result.unwrap_err();
        let err_msg = err.to_string();
        assert!(err_msg.contains("CRC32 checksum mismatch"));
        assert!(err_msg.contains("expected"));
        assert!(err_msg.contains("got"));
    }
}
