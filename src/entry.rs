use std::io;
use std::io::ErrorKind::InvalidData;

use super::{MAX_ENTRY_SIZE, Hash, Value, ValueRef};
use super::crypto::{HashState, Vault, Key, Identity, CryptoError};
use encode;

#[derive(Clone)]
/// A fog-pack value that can be signed and compressed, with an associated document hash and field.
pub struct Entry {
    hash_state: Option<HashState>,
    entry_hash: Option<Hash>,
    hash: Hash,
    doc: Hash,
    field: String,
    entry_len: usize,
    entry: Vec<u8>,
    signed_by: Vec<Identity>,
    compressed: Option<Vec<u8>>
}

// Entries are completely identical (including parent hash and field) if their hashes match
impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        self.hash() == other.hash()
    }
}

impl Eq for Entry {}

// Anything working with Entry in a mutable fashion (or creating one) is responsible for keeping 
// the hashes updated. Namely, `hash_state` must be updated when entry/field/doc change, and 
// entry_hash must be updated if field/doc change or the encoded value changes. The `hash` must 
// always be kept up-to-date.
impl Entry {

    /// Not to be used outside the crate. Allows for creation of an Entry from internal parts. 
    /// Should only be used by Schema/NoSchema for completing the decoding process.
    pub(crate) fn from_parts(
        hash_state: Option<HashState>,
        entry_hash: Option<Hash>,
        hash: Hash,
        doc: Hash,
        field: String,
        entry_len: usize,
        entry: Vec<u8>,
        signed_by: Vec<Identity>,
        compressed: Option<Vec<u8>>,
    ) -> Entry {
        Entry {
            hash_state,
            entry_hash,
            hash,
            doc,
            field,
            entry_len,
            entry,
            signed_by,
            compressed,
        }
    }

    /// Create a new entry from a given Value. Fails if the resulting entry is larger than the 
    /// maximum allowed size.
    pub fn new(doc: Hash, field: String, v: Value) -> Result<Entry, ()> {
        let mut entry = Vec::new();
        encode::write_value(&mut entry, &v);
        let entry_len = entry.len();
        if entry_len > MAX_ENTRY_SIZE {
            return Err(()); // Entry is too big
        }
        let mut entry = Entry {
            doc,
            field,
            hash_state: None,
            entry_hash: None,
            hash: Hash::new_empty(),
            entry_len,
            entry,
            signed_by: Vec::new(),
            compressed: None,
        };
        entry.populate_hash_state();
        Ok(entry)
    }

    pub(crate) fn populate_hash_state(&mut self) {
        let mut temp = Vec::new();
        let mut hash_state = HashState::new(1).unwrap(); // Shouldn't fail if version == 1
        encode::write_value(&mut temp, &Value::from(self.doc.clone()));
        hash_state.update(&temp[..]);
        temp.clear();
        encode::write_value(&mut temp, &Value::from(self.field.clone()));
        hash_state.update(&temp[..]);
        hash_state.update(&self.entry[..self.entry_len]);
        let entry_hash = hash_state.get_hash();
        let hash = if self.entry.len() > self.entry_len {
            hash_state.update(&self.entry[self.entry_len..]);
            hash_state.get_hash()
        } else {
            entry_hash.clone()
        };
        self.hash_state = Some(hash_state);
        self.entry_hash = Some(entry_hash);
        self.hash = hash;
    }

    /// Sign the entry with a given Key from a given Vault.  Fails if the key is invalid 
    /// (`BadKey`), can't be found (`NotInStorage`), or the resulting entry is larger than the 
    /// maximum allowed entry size.
    pub fn sign(&mut self, vault: &Vault, key: &Key) -> Result<(), CryptoError> {
        if self.hash_state.is_none() || self.entry_hash.is_none() {
            self.populate_hash_state();
        }
        let signature = vault.sign(self.entry_hash.as_ref().unwrap(), key)?;
        self.signed_by.push(signature.signed_by().clone());
        let len = self.entry.len();
        signature.encode(&mut self.entry);
        let new_len = self.entry.len();
        if new_len > MAX_ENTRY_SIZE {
            return Err(CryptoError::Io(io::Error::new(InvalidData, "Entry is too large with signature")));
        }
        if new_len > len {
            let hash_state = self.hash_state.as_mut().unwrap();
            hash_state.update(&self.entry[len..]);
            self.hash = hash_state.get_hash();
        }
        self.compressed = None;
        Ok(())
    }

    /// Get an iterator over all known signers of the document.
    pub fn signed_by(&self) -> std::slice::Iter<Identity> {
        self.signed_by.iter()
    }

    /// Get the length of the entry (minus document hash & field name) prior to encoding
    pub fn len(&self) -> usize {
        self.entry.len()
    }

    /// Get the Hash of the Entry as it currently is. Note that adding additional signatures 
    /// will change the Hash.
    pub fn hash(&self) -> &Hash {
        &self.hash
    }

    /// Get the Hash of the parent document for the Entry.
    pub fn doc_hash(&self) -> &Hash {
        &self.doc
    }

    /// Get the field for this entry.
    pub fn field(&self) -> &str {
        self.field.as_str()
    }

    /// Retrieve the value stored inside the entry as a `ValueRef`. This value has the same 
    /// lifetime as the Entry; it can be converted to a `Value` if it needs to outlast the 
    /// Entry.
    pub fn value(&self) -> ValueRef {
        super::decode::read_value_ref(&mut &self.entry[..]).unwrap()
    }

    pub(crate) fn raw_entry(&self) -> &[u8] {
        &self.entry
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{Vault, PasswordLevel};

    fn test_entry() -> Entry {
        let test: Value = fogpack!(vec![0u8, 1u8, 2u8]);
        Entry::new(Hash::new_empty(), String::from(""), test).expect("Should be able to make entry")
    }

    fn prep_vault() -> (Vault, Key) {
        let mut vault = Vault::new_from_password(PasswordLevel::Interactive, "test".to_string())
            .expect("Should have been able to make a new vault for testing");
        let key = vault.new_key();
        (vault, key)
    }

    #[test]
    fn equality_checks() {
        let test0 = Entry::new(Hash::new_empty(), String::from("a"), fogpack!(vec![0u8, 1u8, 2u8]))
            .expect("Should be able to make entry");
        let test1 = test_entry();
        let test2 = test_entry();
        assert!(test0 != test1, "Different entries were considered equal");
        assert!(test2 == test1, "Identically-generated entries weren't considered equal");
    }

    #[test]
    fn large_data() {
        let mut large_bin = Vec::new();
        large_bin.resize(MAX_ENTRY_SIZE, 0u8);
        let test: Value = fogpack!(large_bin.clone());
        let test = Entry::new(Hash::new_empty(), String::from(""), test);
        assert!(test.is_err(), "Should've been too large to encode as a document");

        large_bin.resize(MAX_ENTRY_SIZE-5, 0u8);
        let test: Value = fogpack!(large_bin);
        let test = Entry::new(Hash::new_empty(), String::from(""), test);
        assert!(test.is_ok(), "Should've been just small enough to encode as a document");

        let mut test = test.unwrap();
        let (vault, key) = prep_vault();
        assert!(test.sign(&vault, &key).is_err(), "Should've failed because signing put it past the maximum allowed size");
    }


}
