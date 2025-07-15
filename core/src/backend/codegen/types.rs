#![allow(dead_code)]

const MAGIC: &[u8; 4] = b"DUKA";
const VERSION: u16 = 1;
const SIZE_OF_NUMBER: usize = size_of::<f64>();
const SIZE_OF_INTEGER: usize = size_of::<i64>();

struct FileHeader {
    magic: [u8; 4],
    version: u16,
    num_size: usize,
}
