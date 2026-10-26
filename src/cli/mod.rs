/// Prints a line of human output: on stdout, or on stderr when stdout is
/// reserved for `--json` output.
macro_rules! say {
    ($json:expr) => {
        if $json { eprintln!() } else { println!() }
    };
    ($json:expr, $($arg:tt)*) => {
        if $json { eprintln!($($arg)*) } else { println!($($arg)*) }
    };
}

pub mod add;
pub mod doctor;
pub mod init;
pub mod notes;
pub mod publish;
pub mod status;
pub mod up;
pub mod version;
