#![allow(non_snake_case)]

use duka::{duka_arg, duka_export, duka_ret_int};

extern "C" fn tri_add(L: *mut duka::DukaCall) -> std::ffi::c_int {
    let a = duka_arg(L, 0);
    let b = duka_arg(L, 1);
    let sum = unsafe { a.as_.integer + b.as_.integer };
    duka_ret_int(L, sum);
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn dukaopen_fixture(m: *mut duka::DukaModule) -> std::ffi::c_int {
    duka_export(m, c"tri_add".as_ptr(), Some(tri_add));
    0
}
