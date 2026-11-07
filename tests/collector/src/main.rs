//! A program made of what Lyftar compiled and this collector, linked together with no LLVM (D45, D46).

mod collector;

unsafe extern "C" {
    /// What Lyftar compiled: it says how to find roots, then runs.
    fn program_start();
}

fn main() {
    // SAFETY: the test links this in from the program it compiled, which takes nothing and returns nothing.
    unsafe { program_start() };
    print!("{}", collector::take_output());
}
