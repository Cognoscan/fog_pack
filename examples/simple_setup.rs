extern crate condense_db;

use condense_db::*;

fn main() {
    println!("Start up the system");
    crypto::init().expect("Couldn't initialize random-number generator");
    let db = Db::new();
    let mut vault = crypto::Vault::new_from_password(
        crypto::PasswordLevel::Interactive,
        String::from("BadPassword")).unwrap();

    let mut bin_raw = Vec::new();
    let mut bin: Vec<u8> = Vec::new();
    bin.extend_from_slice(b"Test");
    encode::write_value(&mut bin_raw, &Value::from(bin));
    let bin = decode::read_to_bin_ref(&mut &bin_raw[..], 6);
    println!("{:?}", bin);
    println!("{:?}", bin_raw);

    println!("Generate a new ID");
    let my_key = vault.new_key(); 

    println!("Setting up a simple schema");
    let mut test_schema = Document::new(msgpack!({
        "": Hash::new_empty(),
        "name": "Simple chat schema",
        "required": [
            { "name": "title", "type": "Str", "max_len": 255 },
            { "name": "description", "type": "Str" }
        ],
        "entries": [
            {
                "name": "post",
                "type": "Obj",
                "required": [
                    { "name": "time", "type": "Time" },
                    { "name": "text", "type": "String" }
                ]
            }
        ]
    })).unwrap();
    let schema_permission = Permission::new().local_net(true).direct(true);
    let res = db.add_doc(test_schema, &schema_permission, 0).unwrap();
    let res = res.recv().unwrap();
    println!("    Got back: {:?}", res);

    println!("Making a document");
    let mut test_doc = Document::new(msgpack!({
        "": Hash::new_empty(),
        "title": "Test chat",
        "description": "This is a test chat",
    })).unwrap();
    let doc_permission = schema_permission.clone().advertise(true);
    let doc_hash = test_doc.hash();
    let res = db.add_doc(test_doc, &doc_permission, 0).unwrap();
    let res = res.recv().unwrap();
    println!("    Got back: {:?}", res);

    println!("Making an entry");
    let test_entry = Entry::new_signed(doc_hash.clone(), String::from("post"), msgpack!({
        "time": Timestamp::now().unwrap(),
        "text": "Making a post",
    }), &vault, &my_key).unwrap();
    let res = db.add_entry(test_entry, 0).unwrap();
    let res = res.recv().unwrap();
    println!("    Got back: {:?}", res);

    println!("Retrieving a document");
    let mut query = Query::new();
    query.add_root(&doc_hash);
    let res = db.query(query, &doc_permission, 1).unwrap();
    loop {
        let query_result = res.recv().unwrap();
        match query_result {
            QueryResponse::Doc((doc, effort)) => {
                println!("    Got a document back, effort = {}", effort);
            },
            QueryResponse::Entry(_) => {
                println!("    Got an entry back");
            },
            QueryResponse::Invalid => {
                println!("    Invalid query");
                break;
            }
            QueryResponse::DoneForever => {
                println!("    Done forever");
                break;
            },
            QueryResponse::BadDoc(_) => {
                println!("    BadDoc: One of the root hashes refers to a document that fails schema checks");
            }
            QueryResponse::UnknownSchema(_) => {
                println!("    UnknownSchema: One of the root hashes mapped to a document that has an unrecognized schema");
            }
        };
    }
    drop(res); // Done with query response

    println!("Deleting a document");
    let res = db.del_doc(doc_hash.clone()).unwrap();
    let res = res.recv().unwrap();
    println!("    Got back: {:?}", res);

    println!("Closing the database");
    db.close().unwrap();
}
