use std::io;
use std::io::ErrorKind::InvalidData;
use CompressType;
use super::{MAX_DOC_SIZE, MAX_ENTRY_SIZE, Hash, Document, Entry, Value};
use super::document::parse_schema_hash;
use decode;
use encode;
use crypto;

pub struct NoSchema {
    compress: zstd_safe::CCtx<'static>,
    decompress: zstd_safe::DCtx<'static>,
}

impl NoSchema {
    pub fn new() -> NoSchema {
        NoSchema {
            compress: zstd_safe::create_cctx(),
            decompress: zstd_safe::create_dctx(),
        }
    }

    /// Encode the document and write it to an output buffer.
    pub fn encode_doc(&self, doc: &Document, buf: &mut Vec<u8>) {
        CompressType::Uncompressed.encode(buf);
        let len = doc.len();
        assert!(len <= MAX_DOC_SIZE,
            "Document was larger than maximum size! Document implementation should've made this impossible!");
        buf.extend_from_slice(doc.raw_doc());
    }

    fn compress(&mut self, raw: &[u8], level: i32, buf: &mut Vec<u8>) {
        // Allocate a slightly more space than is in the input
        let vec_len = buf.len();
        let mut buffer_len = zstd_safe::compress_bound(raw.len());
        buf.reserve(buffer_len);
        unsafe {
            buf.set_len(vec_len + buffer_len);
            buffer_len = zstd_safe::compress_cctx(
                &mut self.compress,
                &mut buf[vec_len..],
                raw,
                level
            ).expect("zstd library unexpectedly errored during compress_cctx!");
            buf.set_len(vec_len + buffer_len);
        }
    }


    /// Encode the document, compress it, and write it to an output buffer. The level of 
    /// compression is passed to zstd. 0 will cause it to use the default compression level.
    /// This panics if the underlying zstd calls return an error, which shouldn't be possible with 
    /// the way they are used in this library.
    pub fn compress_doc(&mut self, doc: &Document, level: i32, buf: &mut Vec<u8>) {
        if doc.schema_hash().is_some() {
            CompressType::Compressed.encode(buf);
        }
        else {
            CompressType::CompressedNoSchema.encode(buf);
        }

        let mut raw: &[u8] = doc.raw_doc();

        // Don't encode schema hash if it exists
        if doc.schema_hash().is_some() {
            let _ = parse_schema_hash(&mut raw)
                .expect("Document has invalid vec!")
                .expect("Document has invalid vec!");
            let header_len = doc.raw_doc().len() - raw.len();
            buf.extend_from_slice(&doc.raw_doc()[..header_len]);
        }

        self.compress(raw, level, buf);
    }

    /// Encode an entry and write it to an output buffer. Includes the entry content only, not the 
    /// parent document hash or the field.
    pub fn encode_entry(&self, entry: &Entry, buf: &mut Vec<u8>) {
        CompressType::Uncompressed.encode(buf);
        let len = entry.len();
        assert!(len <= MAX_ENTRY_SIZE,
            "Entry was larger than maximum size! Entry implementation should've made this impossible!");
        buf.extend_from_slice(entry.raw_entry());
    }

    /// Compress an entry and write it to an output buffer. Includes the entry content only, not the 
    /// parent document hash or the field. This panics if the underlying zstd calls return an 
    /// error, which shouldn't be possible with the way this library uses zstd.
    pub fn compress_entry(&mut self, entry: &Entry, level: i32, buf: &mut Vec<u8>) {
        CompressType::CompressedNoSchema.encode(buf);
        self.compress(entry.raw_entry(), level, buf);
    }

    /// Read a document from a byte slice, trusting the origin of the slice and doing as few checks 
    /// as possible when decoding. It fails if there isn't a valid fogpack value, the compression 
    /// isn't recognized/is invalid, the slice terminates early, or if the document is using a 
    /// compression method that requires a schema. The presence of a schema is otherwise not 
    /// checked for.
    ///
    /// Rather than compute the hash, the document hash can optionally be provided. If integrity 
    /// checking is desired, provide no hash and compare the expected hash with the hash of the 
    /// resulting document.
    ///
    /// The *only* time this should be used is if the byte slice is coming from a well-trusted 
    /// location, like an internal database.
    pub fn trusted_decode_doc(&mut self, buf: &mut &[u8], hash: Option<Hash>) -> io::Result<Document> {
        // TODO: Change this function so that it doesn't copy any data until the very end.
        let (doc, compressed) = self.decode_raw(MAX_DOC_SIZE, buf)?;

        // Parse the document itself & optionally start up the hasher
        let doc_len = decode::verify_value(&mut &doc[..])?;

        let (hash_state, doc_hash, hash) = if let Some(hash) = hash {
            (None, None, hash)
        }
        else {
            let mut hash_state = crypto::HashState::new(1).unwrap(); // Shouldn't fail if version == 1
            hash_state.update(&doc[..doc_len]);
            let doc_hash = hash_state.get_hash();
            let hash = if doc.len() > doc_len {
                hash_state.update(&doc[doc_len..]);
                hash_state.get_hash()
            }
            else {
                doc_hash.clone()
            };
            (Some(hash_state), Some(doc_hash), hash)
        };

        // Get signatures
        let mut signed_by = Vec::new();
        let mut index = &mut &doc[doc_len..];
        while index.len() > 0 {
            let signature = crypto::Signature::decode(&mut index)
                .map_err(|_e| io::Error::new(InvalidData, "Invalid signature in raw document"))?;
            signed_by.push(signature.signed_by().clone());
        }

        Ok(Document::from_parts(
            hash_state,
            doc_hash,
            hash,
            doc_len,
            doc,
            compressed,
            signed_by,
            None
        ))
    }

    /// Read a document from a byte slice, performing a full set of validation checks when 
    /// decoding. Success guarantees that the resulting Document is valid, and as such, this can be 
    /// used with untrusted inputs.
    ///
    /// Validation checking means this will fail if:
    /// - The data is corrupted or incomplete
    /// - The data isn't valid fogpack 
    /// - The compression is invalid or expands to larger than the maximum allowed size
    /// - The compression requires the schema to decode
    /// - The decompressed document has an associated schema hash
    /// - Any of the attached signatures are invalid
    pub fn decode_doc(&mut self, buf: &mut &[u8]) -> io::Result<Document> {
        // TODO: Change this function so that it doesn't copy any data until the very end.
        let (doc, compressed) = self.decode_raw(MAX_DOC_SIZE, buf)?;

        // Parse the document itself
        if parse_schema_hash(&mut &doc[..])?.is_some() {
            return Err(io::Error::new(InvalidData, "Document has a schema"));
        }
        let doc_len = decode::verify_value(&mut &doc[..])?;

        // Compute the document hashes
        let mut hash_state = crypto::HashState::new(1).unwrap(); // Shouldn't fail if version == 1
        hash_state.update(&doc[..doc_len]);
        let doc_hash = hash_state.get_hash();
        let hash = if doc.len() > doc_len {
            hash_state.update(&doc[doc_len..]);
            hash_state.get_hash()
        }
        else {
            doc_hash.clone()
        };

        // Get & verify signatures
        let mut signed_by = Vec::new();
        let mut index = &mut &doc[doc_len..];
        while index.len() > 0 {
            let signature = crypto::Signature::decode(&mut index)
                .map_err(|_e| io::Error::new(InvalidData, "Invalid signature in raw document"))?;
            if !signature.verify(&doc_hash) {
                return Err(io::Error::new(InvalidData, "Signature doesn't verify against document"));
            }
            signed_by.push(signature.signed_by().clone());
        }

        Ok(Document::from_parts(
            Some(hash_state),
            Some(doc_hash),
            hash,
            doc_len,
            doc,
            compressed,
            signed_by,
            None
        ))
    }

    /// Read an entry from a byte slice, trusting the origin of the slice and doing as few checks 
    /// as possible when decoding. Functions like [`trusted_decode_doc`], but for entries.
    pub fn trusted_decode_entry(&mut self, buf: &mut &[u8], doc: Hash, field: String, hash: Option<Hash>) -> io::Result<Entry> {
        // TODO: Change this function so that it doesn't copy any data until the very end.
        let (entry, compressed) = self.decode_raw(MAX_ENTRY_SIZE, buf)?;

        // Parse the document itself & load in the optional hash
        let entry_len = decode::verify_value(&mut &entry[..])?;
        let hash_provided = hash.is_some();
        let hash = hash.unwrap_or(Hash::new_empty());

        // Get signatures
        let mut signed_by = Vec::new();
        let mut index = &mut &entry[entry_len..];
        while index.len() > 0 {
            let signature = crypto::Signature::decode(&mut index)
                .map_err(|_e| io::Error::new(InvalidData, "Invalid signature in raw document"))?;
            signed_by.push(signature.signed_by().clone());
        }

        let mut entry = Entry::from_parts(
            None,
            None,
            hash,
            doc,
            field,
            entry_len,
            entry,
            signed_by,
            compressed
        );

        if !hash_provided {
            entry.populate_hash_state();
        }

        Ok(entry)
    }

    /// Read an entry from a byte slice, performing a full set of validation checks when decoding. 
    /// Success guarantees the resulting entry is valid, and as such, this can be used with 
    /// untrusted inputs. Functions like [`decode_doc`]; see its documentation for the possible 
    /// failure cases.
    pub fn decode_entry(&mut self, buf: &mut &[u8], doc: Hash, field: String) -> io::Result<Entry> {
        // TODO: Change this function so that it doesn't copy any data until the very end.
        let (entry, compressed) = self.decode_raw(MAX_ENTRY_SIZE, buf)?;

        let entry_len = decode::verify_value(&mut &entry[..])?;

        let mut temp = Vec::new();
        let mut hash_state = crypto::HashState::new(1).unwrap(); // Shouldn't fail if version == 1
        encode::write_value(&mut temp, &Value::from(doc.clone()));
        hash_state.update(&temp[..]);
        temp.clear();
        encode::write_value(&mut temp, &Value::from(field.clone()));
        hash_state.update(&temp[..]);
        hash_state.update(&entry[..entry_len]);
        let entry_hash = hash_state.get_hash();
        let hash = if entry.len() > entry_len {
            hash_state.update(&entry[entry_len..]);
            hash_state.get_hash()
        } else {
            entry_hash.clone()
        };

        // Get signatures
        let mut signed_by = Vec::new();
        let mut index = &mut &entry[entry_len..];
        while index.len() > 0 {
            let signature = crypto::Signature::decode(&mut index)
                .map_err(|_e| io::Error::new(InvalidData, "Invalid signature in raw entry"))?;
            if !signature.verify(&entry_hash) {
                return Err(io::Error::new(InvalidData, "Signature doesn't verify against entry"));
            }
            signed_by.push(signature.signed_by().clone());
        }

        Ok(Entry::from_parts(
            Some(hash_state),
            Some(entry_hash),
            hash,
            doc,
            field,
            entry_len,
            entry,
            signed_by,
            compressed
        ))
    }

    fn decode_raw(&mut self, max_size: usize, buf: &mut &[u8]) -> io::Result<(Vec<u8>, Option<Vec<u8>>)> {
        let compress_type = CompressType::decode(buf)?;
        match compress_type {
            CompressType::Uncompressed => {
                if buf.len() > max_size {
                    return Err(io::Error::new(InvalidData, "Data is larger than maximum allowed size"));
                }
                let mut doc = Vec::new();
                doc.extend_from_slice(buf);
                Ok((doc, None))
            },
            CompressType::CompressedNoSchema => {
                let mut compressed = Vec::new();
                // Save off the compressed data
                compress_type.encode(&mut compressed);
                compressed.extend_from_slice(buf);
                // Decompress the data
                // Find the expected size, and fail if it's larger than the maximum allowed size.
                let expected_len = zstd_safe::get_frame_content_size(buf);
                if expected_len > (max_size as u64) {
                    return Err(io::Error::new(InvalidData, "Expected decompressed size is larger than maximum allowed size"));
                }
                let expected_len = expected_len as usize;
                let mut doc = Vec::with_capacity(expected_len);
                unsafe {
                    doc.set_len(expected_len);
                    let len = zstd_safe::decompress_dctx(
                        &mut self.decompress,
                        &mut doc[..],
                        buf
                    ).map_err(|_| io::Error::new(InvalidData, "Decompression failed"))?;
                    doc.set_len(len);
                }
                Ok((doc, Some(compressed)))
            },
            CompressType::Compressed | CompressType::DictCompressed => {
                return Err(io::Error::new(InvalidData, "Data uses a schema, but NoSchema struct was used for decoding"));
            },
        }
    }

}

fn _assert_traits() {
    fn _assert_send<T: Send>(_: T) {}
    _assert_send(NoSchema::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::Value;
    use crate::crypto::{Vault, PasswordLevel, Key};

    fn test_doc() -> Document {
        let test: Value = fogpack!({
            "test": true,
            "boolean": true,
            "positive": 1,
            "negative": -1,
            "string": "string",
            "float32": 1.0f32,
            "float64": 1.0f64,
            "binary": vec![0u8,1u8,2u8],
            "array": [Value::from(0), Value::from("an_array")] 
        });
        Document::new(test).expect("Should've been able to encode as a document")
    }

    fn test_doc_with_schema() -> Document {
        let fake_hash = Hash::new(1, "test".as_bytes()).expect("Should've been able to make hash");
        let test: Value = fogpack!({
            "" : fake_hash,
            "test": true,
            "boolean": true,
        });
        Document::new(test).expect("Should've been able to encode as a document")
    }

    fn test_entry(doc: Hash, field: String) -> Entry {
        let test: Value = fogpack!(vec![0u8, 1u8, 2u8]);
        Entry::new(doc, field, test).expect("Should've been able to encode as an entry")
    }

    #[test]
    fn encode_decode() {
        let test = test_doc();
        let mut schema_none = NoSchema::new();
        let mut enc = Vec::new();
        schema_none.encode_doc(&test, &mut enc);
        let dec = schema_none.trusted_decode_doc(&mut &enc[..], None).expect("Decoding should have worked");
        let mut enc2 = Vec::new();
        schema_none.encode_doc(&dec, &mut enc2);
        assert!(test == dec, "Encode->Decode should yield same document");
        assert!(enc == enc2, "Encode->Decode->encode didn't yield identical results");
    }

    #[test]
    fn compress_decompress() {
        let test = test_doc();
        let mut schema_none = NoSchema::new();
        let mut enc = Vec::new();
        schema_none.compress_doc(&test, 3, &mut enc);
        let dec = schema_none.trusted_decode_doc(&mut &enc[..], None).expect("Decoding should have worked");
        let mut enc2 = Vec::new();
        schema_none.encode_doc(&dec, &mut enc2);
        assert!(test == dec, "Compress->Decode should yield same document");
    }

    fn prep_vault() -> (Vault, Key) {
        let mut vault = Vault::new_from_password(PasswordLevel::Interactive, "test".to_string())
            .expect("Should have been able to make a new vault for testing");
        let key = vault.new_key();
        (vault, key)
    }

    #[test]
    fn compress_decompress_sign() {
        let mut test = test_doc();
        let (mut vault, key0) = prep_vault();
        let key1 = vault.new_key();
        let key2 = vault.new_key();
        test.sign(&vault, &key0).expect("Should have been able to sign test document w/ key0");
        test.sign(&vault, &key1).expect("Should have been able to sign test document w/ key1");
        let mut schema_none = NoSchema::new();
        let mut enc = Vec::new();
        schema_none.compress_doc(&test, 3, &mut enc);
        let mut dec = schema_none.trusted_decode_doc(&mut &enc[..], None).expect("Decoding should have worked");
        test.sign(&vault, &key2).expect("Should have been able to sign test document w/ key2");
        dec.sign(&vault, &key2).expect("Should have been able to sign decoded document w/ key2");
        assert!(test == dec, "Compress->Decode should yield same document, even after signing");
    }

    #[test]
    fn compress_sign_existing_hash() {
        let mut test = test_doc();
        let (vault, key) = prep_vault();
        let mut schema_none = NoSchema::new();
        let mut enc = Vec::new();
        schema_none.compress_doc(&test, 3, &mut enc);
        let mut dec = schema_none.trusted_decode_doc(&mut &enc[..], Some(test.hash().clone())).expect("Decoding should have worked");
        test.sign(&vault, &key).expect("Should have been able to sign test document");
        dec.sign(&vault, &key).expect("Should have been able to sign decoded document");
        assert!(test == dec, "Compress->Decode should yield same document, even after signing");
    }

    #[test]
    fn compress_schema_decode_fails() {
        let test = test_doc_with_schema();
        let mut schema_none = NoSchema::new();
        let mut enc = Vec::new();
        schema_none.compress_doc(&test, 3, &mut enc);
        let dec = schema_none.trusted_decode_doc(&mut &enc[..], Some(test.hash().clone()));
        assert!(dec.is_err(), "Decompression should have failed, as a schema was in the document");
    }

    #[test]
    fn strict_decode_tests() {
        // Prep encode/decode & byte vector
        let mut schema_none = NoSchema::new();
        let mut enc = Vec::new();

        // Prep schema-using document
        let test = test_doc_with_schema();

        schema_none.encode_doc(&test, &mut enc);
        let dec = schema_none.decode_doc(&mut &enc[..]);
        assert!(dec.is_err(), "Decoding should have failed when a schema was in the document");

        enc.clear();
        schema_none.compress_doc(&test, 3, &mut enc);
        let dec = schema_none.decode_doc(&mut &enc[..]);
        assert!(dec.is_err(), "Decompression should have failed when a schema was in the document");

        // Prep new non-schema document with signature
        let (vault, key) = prep_vault();
        let mut test = test_doc();
        test.sign(&vault, &key).expect("Should have been able to sign test document");

        enc.clear();
        schema_none.encode_doc(&test, &mut enc);
        let dec = schema_none.decode_doc(&mut &enc[..]);
        assert!(dec.is_ok(), "Decoding a valid document should have succeeded");
        
    }

    #[test]
    fn corrupted_data_tests() {
        // Prep encode/decode & byte vector
        let mut schema_none = NoSchema::new();
        let mut enc = Vec::new();
        // Prep a non-schema document with a signature
        let (vault, key) = prep_vault();
        let mut test = test_doc();
        test.sign(&vault, &key).expect("Should have been able to sign test document");

        schema_none.encode_doc(&test, &mut enc);
        *(enc.last_mut().unwrap()) = 0;
        let dec = schema_none.decode_doc(&mut &enc[..]);
        assert!(dec.is_err(), "Document signature was corrupted, but decoding succeeded anyway");

        enc.clear();
        schema_none.encode_doc(&test, &mut enc);
        enc[10] = 0xFF;
        let dec = schema_none.decode_doc(&mut &enc[..]);
        assert!(dec.is_err(), "Document payload was corrupted, but decoding succeeded anyway");

        enc.clear();
        schema_none.encode_doc(&test, &mut enc);
        enc[0] = 0x1;
        let dec = schema_none.decode_doc(&mut &enc[..]);
        assert!(dec.is_err(), "Document payload was corrupted, but decoding succeeded anyway");
    }


}
