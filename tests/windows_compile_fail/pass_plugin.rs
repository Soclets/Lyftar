// A plugin doesn't compile on Windows (D44). `examples/kayo_passes_plugin.rs` is the same thing where it does.

fn register(_builder: &mut lyftar::passes::PassBuilder<'static>) {}

lyftar::pass_plugin!("plugin", "0.1.0", register);

fn main() {}
