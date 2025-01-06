import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/l10n.dart';
import '../../utils/settings_page_padding.dart';
import '../../widgets/unavailable_page_on_band.dart';
import '../../widgets/settings/settings_box_scrobble_login.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';

import 'widgets/queue_mode_setting.dart';
import 'widgets/playback_mode_setting.dart';
import 'widgets/adaptive_switching_setting.dart';
import 'widgets/middle_click_action_setting.dart';

class SettingsPlayback extends StatefulWidget {
  const SettingsPlayback({super.key});

  @override
  State<SettingsPlayback> createState() => _SettingsPlaybackState();
}

class _SettingsPlaybackState extends State<SettingsPlayback> {
  @override
  Widget build(BuildContext context) {
    final s = S.of(context);

    return PageContentFrame(
      child: UnavailablePageOnBand(
        child: SingleChildScrollView(
          padding: getScrollContainerPadding(context),
          child: SettingsPagePadding(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                QueueModeSetting(),
                MiddleClickActionSetting(),
                PlaybackModeSetting(),
                AdaptiveSwitchingSetting(),
                Padding(
                  padding:
                      EdgeInsets.only(top: 8, bottom: 2, left: 6, right: 6),
                  child: Text(s.onlineServices),
                ),
                SettingsBoxScrobbleLogin(
                  title: "Last.fm",
                  subtitle: s.lastFmSubtitle,
                  serviceId: 'LastFm',
                ),
                SettingsBoxScrobbleLogin(
                  title: "Libre.fm",
                  subtitle: s.libreFmSubtitle,
                  serviceId: 'LibreFm',
                ),
                SettingsBoxScrobbleLogin(
                  title: "ListenBrainz",
                  subtitle: s.listenBrainzSubtitle,
                  serviceId: 'ListenBrainz',
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
