use anyhow::Result;
use vergen_git2::{BuildBuilder, Emitter, Git2Builder, RustcBuilder};

fn main() -> Result<()> {
    let git2 = Git2Builder::all_git()?;
    let build = BuildBuilder::all_build()?;
    let rustc = RustcBuilder::all_rustc()?;

    Emitter::default()
        .add_instructions(&build)?
        .add_instructions(&git2)?
        .add_instructions(&rustc)?
        .emit()?;

    Ok(())
}
