//! Shared Types & Utils for Duka
//!
//! Including AST, token, compile-time value, constants, errors, utils

use duka_macros::史書云;

use crate::utils::SemVer;

pub mod builtin;
pub mod config;
pub mod constants;
pub mod docs;
pub mod dtype;
pub mod errors;
pub mod ir;
pub mod module;
pub mod types;
pub mod utils;
pub mod value;

pub const VERSION: SemVer = 史書云! {
    <<共有>> 者
    為 世家 "項目之創立" 也
    為 世家 "優化" 也
};

#[cfg(test)]
mod tests {

    #[test]
    fn semver_test() {
        use crate::utils::SemVer;

        let ver = SemVer {
            major: 1,
            minor: 21,
            patch: 2,
        };
        let ver2 = SemVer {
            major: 0,
            minor: 35,
            patch: 4,
        };
        assert!(ver > ver2)
    }
}
