# Privacy Policy

**Last Updated: November 22, 2026**

**Introduction**

This Privacy Policy outlines how Rune (Rune Player) ("we," "our," or "the application") manages information. We prioritize your privacy and are committed to transparency.

**Information Collection**

You can play a local music library without registering a Rune account. Rune does not collect usage statistics, track your behavior for analytics, or use cookies or similar technologies.

**Local Data Storage**

Rune stores your music library information, playback preferences, custom playlists and analysis results on your device. Rune uses the library ID to identify the source of a track locally. It never sends this ID to a scrobbling service.

**Third-Party Services**

Rune does not include third-party advertising or analytics. Scrobbling is optional. In **Settings → Playback**, you can connect a service to share what you are listening to. You can log out of each service there to stop reporting new listens to that account.

Each connected service receives the artist and track title in both now-playing updates and recorded listens (scrobbles). Both kinds of report include the album name when available. A recorded listen also includes a timestamp.

- **Last.fm** receives your username and password at login and returns a session key for playback reports. Rune includes your Last.fm API key in each request and uses your API secret locally to sign it. Both kinds of report can include the album artist. Only now-playing updates include the track duration when available.
- **Libre.fm** receives your username and password at login, along with an authentication token derived from both. It returns a session key for playback reports. Rune includes a fixed API key in each request. Both kinds of report can include the album artist. Only now-playing updates include the track duration when available.
- **ListenBrainz** receives your user token in the authorization header when Rune validates the token and when it submits playback reports. Both kinds of report identify the media player as `rune` and include the track duration when available. Rune omits the album artist from ListenBrainz reports.

**Data Security**

Your device's security settings help protect locally stored information. Rune sends scrobbling credentials and playback reports to the selected service over HTTPS. The service handles the information it receives under its own privacy policy.

**Children's Privacy**

If a child connects a scrobbling account, the selected service receives the information described above.

**Changes to This Privacy Policy**

We may update this Privacy Policy as the application evolves. Users will be notified of any significant changes through the application or our GitHub repository.

**Your Rights Under GDPR and CCPA**

To access, correct, delete or export listening data sent to a scrobbling service, or to restrict or object to its use, contact that service. For questions or requests about data handled by Rune, use the contact below.

**Contact Information**

For privacy-related questions or concerns, please contact us through our [GitHub repository](https://github.com/losses/rune).
