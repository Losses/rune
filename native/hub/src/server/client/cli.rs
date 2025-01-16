use clap::{CommandFactory, FromArgMatches, Parser};

#[derive(Debug, Parser)]
#[command(name = "rune-speaker")]
pub enum Command {
    /// List contents of current directory
    Ls,
    /// Print current working directory
    Pwd,
    /// Change directory
    Cd {
        /// Directory to change to
        path: String,
    },
    /// Show help information
    Help,
    /// Exit the program
    Quit,
}

impl Command {
    pub fn parse(input: &str) -> Result<Self, clap::Error> {
        let input_vec: Vec<String> = std::iter::once("".to_string())
            .chain(shlex::split(input).unwrap_or_default())
            .collect();

        let args = input_vec.iter().map(|s| s.as_str());

        let matches = Command::command()
            .override_usage("> [COMMAND]")
            .disable_help_flag(true)
            .try_get_matches_from(args)?;

        Command::from_arg_matches(&matches)
    }
}
