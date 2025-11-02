/// This key is used to store and retrieve the paths of files that have been
/// previously opened by the user. This allows the application to maintain a
/// history of accessed files and provide quick access to recently used content
/// across sessions.
const kOpenedFilesKey = 'library_path';

/// The purpose of this key is to track the version of the stored file path data
/// structure. When the application's data model changes, this version number is
/// incremented, enabling seamless migration of user data from older formats to
/// newer ones without data loss.
const kDataVersionKey = 'library_path_version';

/// The key is used to check if there is a predefined layout mode that the user
/// or application has set, which should take precedence over the dynamically
/// calculated device type based on screen size and orientation.
const kForceLayoutModeKey = 'force_layout_mode';

/// This key is utilized to store and retrieve the index of the last played
/// item in a playlist, ensuring that playback can resume from the correct
/// position after Rune is restarted or a session is resumed.
const kLastQueueIndexKey = 'last_queue_index';

/// The primary purpose of this key is to serve as an identifier for storing
/// and retrieving the user's choice of computing device (e.g., CPU or GPU) from
/// persistent storage.
/// The setting has been hidden due to significant stability issues.
const kAnalysisComputingDeviceKey = 'analysis_mode';

/// This key is used to adjust the workloadFactor based on the user's performance
/// level preference, which is then used to determine the batch size for
/// processing tasks by multiplying the number of CPU cores with a workload factor
/// and constraining the result between predefined minimum and maximum limits.
const kAnalysisPerformanceLevelKey = 'analysis_performance';

/// The primary purpose of this key is to provide a mechanism for persisting user
/// settings regarding which playback modes should be disabled.
const kDisabledPlaybackModesKey = 'disabled_playback_modes';

/// The primary purpose of this key is to provide flexibility and customization
/// for users, allowing them to define what action should be executed when they
/// middle-click on an item within Rune.
const kMiddleClickActionKey = 'middle_click_action';

/// The primary purpose of this key is to enable or disable adaptive switching
/// in the player's configuration. This feature is particularly useful for
/// scenarios where users want the playback to adapt dynamically, such as replaying
/// a track if it was accidentally skipped or switching back to a previous track
/// under specific conditions.
const kAdaptiveSwitchingKey = 'adaptive_switching';

/// This key is used to store the user's preference for the color mode of the
/// application. This can include options such as "system", "dark", or "light".
const kColorModeKey = 'color_mode';

/// This key is used to store the user's selected theme color. This color is
/// used to customize Rune's visual elements, providing a personalized
/// aesthetic experience.
const kThemeColorKey = 'theme_color';

/// The primary function of this key is to control the display of branding
/// animations during Rune's startup.
const kDisableBrandingAnimationKey = 'disable_branding_animation';

/// When dynamic colors are enabled, Rune fetches the primary color from
/// the cover art of the currently playing track and applies it to Rune's theme.
const kEnableDynamicColorsKey = 'enable_dynamic_color';

/// This key is used to determine how window should be sized upon startup. If no
/// specific window size mode is stored, Rune defaults to a 'normal' window size
/// mode.
const kWindowSizeKey = 'window_size';

/// This key allows Rune to recall the window size set by the user during their
/// last session.
const kRememberWindowSizeKey = 'remember_window_size';

/// The primary function of this key is to remember the last window size set by
/// the user. By storing this information, Rune can restore the window to the same
/// size when it is reopened.
const kRememberdWindowSizeKey = 'rememberd_window_size';

/// The primary purpose of this key is to ensure that Rune can remember and apply
/// the user's preferred language and regional settings across sessions.
const kLocaleKey = 'locale';

/// This key is used to store the user's preferred playback mode, which can be
/// retrieved and applied when the user starts Rune.
const kPlaybackModeKey = 'playback_mode';

/// This key is integral to determining how new items are added to the playback
/// queue without replacing the current playlist.
const kNonReplaceOperateModeKey = 'playlist_operate_mode';

/// The primary purpose of this key is to allow customization of the startup
/// sound effect that plays when Rune launches.
const kBandingSfxKey = 'branding_sfx';

/// The primary purpose of this key is to allow users or developers to specify
/// how many cover art images should be fetched and displayed on the cover wall.
const kRandomCoverWallCountKey = 'random_cover_wall_count';

/// In Cafe Mode, the application automatically navigates to the "Cover Art Wall"
/// upon startup, displaying album artwork for an enhanced visual experience.
/// Additionally, it initiates music playback using predefined queries, creating a
/// seamless audio-visual environment ideal for settings like cafes or public
/// spaces.
const kCafeModeKey = 'cafe_mode';

/// This key is used to tweak the animation speed and intensity of the spectrum
/// visualizer.
const kMildSpectrumKey = 'mild_spectrum';

/// The primary purpose of this key is to configure a human-readable identifier
/// for this device during Neighbor Discovery, allowing users to recognize their
/// own device in network lists through a friendly name rather than technical identifiers.
/// This alias will be broadcasted to nearby peers and displayed in discovery UIs.
const kDeviceAliasKey = 'device_alias';

/// This key stores the fingerprint of the SSL public certificate used to authenticate
/// secure communication channels during Neighbor Discovery. The fingerprint serves
/// as a trust anchor for peer verification, ensuring connections are established only
/// with cryptographically validated devices.
const kFingerprintKey = 'device_fingerprint';

/// This key is used to track whether the minimization notification has been
/// shown to the user. When a user closes the application window for the first time
/// and it minimizes to the system tray instead of terminating, a notification
/// is displayed to inform the user of this behavior. Setting this flag prevents
/// the notification from appearing on subsequent window closes, providing a
/// better user experience by avoiding repetitive notifications.
const kCloseNotificationShownKey = 'close_notification_shown';

/// The primary purpose of this key is to determine the behavior when a user
/// clicks the close button on the application window. It can be configured to
/// either minimize the application to the system tray or completely close the
/// window and terminate the application.
const kClosingWindowBehaviorKey = 'closing_window_behavior';

/// This key is used to configure the threshold value for audio similarity analysis.
/// It determines the minimum similarity score required for two audio tracks to be
/// considered related or matching. A higher threshold results in stricter matching
/// criteria, reducing false positives but potentially missing similar tracks, while
/// a lower threshold increases detection sensitivity at the cost of potential false
/// matches.
const kDeduplicateThresholdKey = 'deduplicate_threshold';

/// This key is used to enable or disable custom window controls on Linux.
/// When enabled, the application displays a frameless window with custom window
/// controls similar to the Windows implementation. For compatibility reasons,
/// this defaults to false (disabled) when the setting is missing.
const kLinuxCustomWindowControlsKey = 'linux_custom_window_controls';

/// This key is used to store the user's preference for the tray icon color mode.
/// It determines whether the tray icon should automatically adapt to the system
/// theme or use a fixed color mode. Options include "auto" (follows system theme),
/// "light" (always use light icon), and "dark" (always use dark icon).
const kTrayIconColorModeKey = 'tray_icon_color_mode';
