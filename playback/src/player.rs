use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use tokio::sync::mpsc;

use crate::internal::{PlayerCommand, PlayerEvent, PlayerInternal};

// Define the Player struct, which includes a channel sender for sending commands
pub struct Player {
    commands: Arc<Mutex<mpsc::UnboundedSender<PlayerCommand>>>,
}

impl Player {
    // Create a new Player instance and return the Player and the event receiver
    pub fn new() -> (Self, mpsc::UnboundedReceiver<PlayerEvent>) {
        // Create an unbounded channel for sending commands
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        // Create an unbounded channel for receiving events
        let (event_sender, event_receiver) = mpsc::unbounded_channel();

        // Create the Player instance and wrap the command sender in Arc<Mutex>
        let player = Player {
            commands: Arc::new(Mutex::new(cmd_tx)),
        };

        // Start a new thread to run the PlayerInternal logic
        thread::spawn(move || {
            // Create a PlayerInternal instance, passing in the command receiver and event sender
            let mut internal = PlayerInternal::new(cmd_rx, event_sender);
            // Create a new Tokio runtime for asynchronous tasks
            let runtime = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
            // Run the main loop of PlayerInternal within the Tokio runtime
            runtime.block_on(internal.run());
        });

        // Return the Player instance and the event receiver
        (player, event_receiver)
    }

    // Send a command to the internal player
    pub fn command(&self, cmd: PlayerCommand) {
        // Acquire the lock and send the command
        if let Ok(commands) = self.commands.lock() {
            commands.send(cmd).unwrap();
        }
    }
}

impl Player {
    pub fn load(&self, index: usize) {
        self.command(PlayerCommand::Load { index });
    }

    pub fn play(&self) {
        self.command(PlayerCommand::Play);
    }

    pub fn pause(&self) {
        self.command(PlayerCommand::Pause);
    }

    pub fn stop(&self) {
        self.command(PlayerCommand::Stop);
    }

    pub fn next(&self) {
        self.command(PlayerCommand::Next);
    }

    pub fn previous(&self) {
        self.command(PlayerCommand::Previous);
    }

    pub fn seek(&self, position_ms: u32) {
        self.command(PlayerCommand::Seek(position_ms));
    }

    pub fn add_to_playlist(&self, path: PathBuf) {
        self.command(PlayerCommand::AddToPlaylist { path });
    }

    pub fn remove_from_playlist(&self, index: usize) {
        self.command(PlayerCommand::RemoveFromPlaylist { index });
    }

    pub fn clear_playlist(&self) {
        self.command(PlayerCommand::ClearPlaylist);
    }
}
