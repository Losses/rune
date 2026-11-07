use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};

#[derive(Clone, Debug)]
pub enum PlaybackMode {
    Sequential,
    RepeatOne,
    RepeatAll,
    Shuffle,
    NoChange,
}

impl std::str::FromStr for PlaybackMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "sequential" => Ok(PlaybackMode::Sequential),
            "repeatone" => Ok(PlaybackMode::RepeatOne),
            "repeatall" => Ok(PlaybackMode::RepeatAll),
            "shuffle" => Ok(PlaybackMode::Shuffle),
            "nochange" => Ok(PlaybackMode::NoChange),
            _ => Err(format!("Unknown playback mode: {s}")),
        }
    }
}

impl From<PlaybackMode> for u32 {
    fn from(val: PlaybackMode) -> Self {
        match val {
            PlaybackMode::Sequential => 0,
            PlaybackMode::RepeatOne => 1,
            PlaybackMode::RepeatAll => 2,
            PlaybackMode::Shuffle => 3,
            PlaybackMode::NoChange => 99,
        }
    }
}

#[derive(Clone, Debug)]
pub enum OperateMode {
    AppendToEnd,
    PlayNext,
    Replace,
}

impl std::str::FromStr for OperateMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "append" | "appendtoend" => Ok(OperateMode::AppendToEnd),
            "next" | "playnext" => Ok(OperateMode::PlayNext),
            "replace" => Ok(OperateMode::Replace),
            _ => Err(format!("Unknown operate mode: {s}")),
        }
    }
}

impl From<OperateMode> for i32 {
    fn from(val: OperateMode) -> Self {
        match val {
            OperateMode::AppendToEnd => 0,
            OperateMode::PlayNext => 1,
            OperateMode::Replace => 2,
        }
    }
}

#[derive(Debug, Parser)]
pub enum ReplCommand {
    /// List contents of current directory
    Ls {
        /// Use long listing format
        #[arg(short = 'l', default_value_t = false)]
        long: bool,
    },
    /// Print current working directory
    Pwd,
    /// Change directory
    Cd {
        /// Directory to change to
        path: String,
        #[arg(long)]
        id: bool,
    },
    /// Alias for `cd --id`
    Cdi {
        /// Directory to change to
        path: String,
    },
    /// Operate playback with mix query
    Opq {
        /// Path to create query from
        path: String,
        /// Playback mode (sequential, repeatone, repeatall, shuffle, nochange)
        #[arg(long, default_value = "nochange")]
        playback_mode: PlaybackMode,
        /// Whether to start playing instantly
        #[arg(long, default_value_t = true)]
        instant_play: bool,
        /// Operation mode (append, next, replace)
        #[arg(long, default_value = "append")]
        operate_mode: OperateMode,
        #[arg(long)]
        id: bool,
    },
    /// Alias for `opq --id`
    Opqi {
        /// Path to create query from
        path: String,
        /// Playback mode (sequential, repeatone, repeatall, shuffle, nochange)
        #[arg(long, default_value = "nochange")]
        playback_mode: PlaybackMode,
        /// Whether to start playing instantly
        #[arg(long, default_value_t = true)]
        instant_play: bool,
        /// Operation mode (append, next, replace)
        #[arg(long, default_value = "append")]
        operate_mode: OperateMode,
    },
    /// Play the current track
    Play,
    /// Pause the current track
    Pause,
    /// Skip to the next track
    Next,
    /// Go back to the previous track
    Previous,
    /// Set playback mode
    SetMode {
        /// Playback mode (sequential, repeatone, repeatall, shuffle)
        mode: PlaybackMode,
    },
    /// Exit the program
    Quit,
    /// Alias for `quit`
    Exit,
}

impl ReplCommand {
    pub fn parse(input: &str) -> Result<Self, clap::Error> {
        let input_vec: Vec<String> = std::iter::once("".to_string())
            .chain(shlex::split(input).unwrap_or_default())
            .collect();

        let args = input_vec.iter().map(|s| s.as_str());

        let matches = ReplCommand::command()
            .override_usage("> [COMMAND]")
            .try_get_matches_from(args)?;

        let command = ReplCommand::from_arg_matches(&matches)?;

        // Convert aliases
        Ok(match command {
            ReplCommand::Cdi { path } => ReplCommand::Cd { path, id: true },
            ReplCommand::Exit => ReplCommand::Quit,
            ReplCommand::Opqi {
                path,
                playback_mode,
                instant_play,
                operate_mode,
            } => ReplCommand::Opq {
                path,
                id: true,
                playback_mode,
                instant_play,
                operate_mode,
            },
            _ => command,
        })
    }
}

#[derive(Debug, Subcommand)]
pub enum DiscoveryCmd {
    /// Scan for devices in the network
    Scan,
    /// List discovered devices
    Ls,
    /// View device certificate information
    Inspect {
        /// Device index
        index: usize,
    },
    /// Trust specified device
    Trust {
        /// Device index
        index: usize,
        /// Trusted hosts (splitted by comma)
        #[arg(long)]
        domains: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum RemoteCmd {
    /// List trusted devices
    Ls,
    /// Delete trusted device
    Untrust {
        /// Certificate index
        index: usize,
    },
    /// Edit device associated hostnames
    Edit {
        /// Certificate fingerprint
        fingerprint: String,
        /// New hostname list (comma-separated)
        hosts: String,
    },
    /// Verify servers by certificate index
    Verify { index: usize },
    /// Fetch device information of the remote revice
    Inspect { host: String },
    /// Register this client to the remote server
    Register { host: String },
    /// Printing the certification summary of this device
    SelfInfo,
}

#[derive(Debug, Parser)]
#[command(name = "rune-client", version, about, long_about = None)]
pub enum Cli {
    /// Interactive REPL mode
    Repl(ReplArgs),
    /// Device discovery and management
    #[command(subcommand)]
    Discovery(DiscoveryCmd),
    /// Remote device trust management
    #[command(subcommand)]
    Remote(RemoteCmd),
}

#[derive(Debug, clap::Args)]
pub struct ReplArgs {
    /// Service URL
    #[arg(help = "The URL of the service, e.g., example.com:7863 or 192.168.1.1:8963")]
    pub service_url: String,
}
