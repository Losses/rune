use anyhow::Result;
use clap::{Arg, Command};
use rand::Rng;
use scrobbling::{
    ScrobblingClient, ScrobblingTrack, last_fm::LastFmClient, libre_fm::LibreFmClient,
    listen_brainz::ListenBrainzClient,
};

#[tokio::main]
async fn main() -> Result<()> {
    let matches = Command::new("Scrobbling Testing CLI")
        .version("1.0")
        .about("Test scrobbling to Last.fm, Libre.fm or ListenBrainz")
        .arg(
            Arg::new("service")
                .help("The service to use (lastfm, librefm, or listenbrainz)")
                .required(true)
                .index(1),
        )
        .arg(
            Arg::new("username")
                .help("Username for the service")
                .required(true)
                .index(2),
        )
        .arg(
            Arg::new("password")
                .help("Password for the service")
                .required(true)
                .index(3),
        )
        .arg(
            Arg::new("api_key")
                .help("API Key for the service")
                .long("api_key"),
        )
        .arg(
            Arg::new("api_secret")
                .help("API Secret for Last.fm (ignored for Libre.fm)")
                .long("api_secret"),
        )
        .arg(
            Arg::new("action")
                .help("Action to perform (nowplaying or scrobble)")
                .required(true)
                .index(4),
        )
        .get_matches();

    let binding = "".to_string();
    let service = matches.get_one::<String>("service").unwrap();
    let api_key = matches.get_one::<String>("api_key").unwrap_or(&binding);
    let api_secret = matches.get_one::<String>("api_secret").unwrap_or(&binding);
    let username = matches.get_one::<String>("username").unwrap();
    let password = matches.get_one::<String>("password").unwrap();
    let action = matches.get_one::<String>("action").unwrap();

    let track = ScrobblingTrack {
        artist: "Random Artist".to_string(),
        track: "Random Track".to_string(),
        album: Some("Random Album".to_string()),
        album_artist: Some("Random Album Artist".to_string()),
        duration: Some(rand::thread_rng().gen_range(180..300)),
        timestamp: None,
    };

    let response = match service.as_str() {
        "lastfm" => {
            let mut client = LastFmClient::new(api_key.to_string(), api_secret.to_string())?;
            client.authenticate(username, password).await?;
            if action == "nowplaying" {
                client.update_now_playing(&track).await?
            } else {
                client.scrobble(&track).await?
            }
        }
        "librefm" => {
            let mut client = LibreFmClient::new()?;
            client.authenticate(username, password).await?;
            if action == "nowplaying" {
                client.update_now_playing(&track).await?
            } else {
                client.scrobble(&track).await?
            }
        }
        "listenbrainz" => {
            let mut client = ListenBrainzClient::new()?;
            client.authenticate(username, password).await?;
            if action == "nowplaying" {
                client.update_now_playing(&track).await?
            } else {
                client.scrobble(&track).await?
            }
        }
        _ => {
            eprintln!("Unsupported service: {}", { service });
            return Ok(());
        }
    };

    println!("Response: {:?}", response.text().await?);

    Ok(())
}
