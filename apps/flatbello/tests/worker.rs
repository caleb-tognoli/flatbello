//! The real `flatbello --worker` process: answers framed requests, and a crash ends only it.

// Test-only code: failing loudly is the point.
#![allow(clippy::unwrap_used, clippy::panic, clippy::expect_used)]

use std::io::{BufReader, BufWriter};
use std::process::{Command, Stdio};

use flatbello_core::FaceId;
use flatbello_engine::wire::{
    Request, Response, Settings, decode_response, encode_request, read_frame, write_frame,
};

fn fixture(name: &str) -> Vec<u8> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/generated")
        .join(name);
    std::fs::read(path).unwrap()
}

#[test]
fn worker_answers_and_dies_alone() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_flatbello"))
        .arg("--worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = BufWriter::new(child.stdin.take().unwrap());
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut call = |r: &Request| {
        write_frame(&mut input, &encode_request(r)).unwrap();
        read_frame(&mut output)
            .unwrap()
            .map(|f| decode_response(&f).unwrap())
    };

    let opened = call(&Request::Open {
        bytes: fixture("plate_holes.step"),
        hint: "plate_holes.step".into(),
        units: None,
    });
    assert!(matches!(opened, Some(Response::Opened(s)) if !s.faces.is_empty()));
    let exported = call(&Request::Export {
        faces: vec![FaceId(0)],
        format: "dxf-r2000".into(),
        settings: Settings::default(),
    });
    assert!(matches!(exported, Some(Response::Exported(b)) if b.starts_with(b"  0\r\nSECTION")));

    // The crash hook aborts the worker: no reply, a failed exit status.
    assert!(call(&Request::Crash).is_none());
    assert!(!child.wait().unwrap().success());
}
