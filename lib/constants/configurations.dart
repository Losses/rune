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
