use octavo::digest::prelude::{Digest, Md5};

pub fn md5(data: &[u8]) -> [u8; 16] {
    let mut md5 = Md5::default();
    md5.update(data);
    let mut result = [0; 16];
    md5.result(&mut result);
    result
}
