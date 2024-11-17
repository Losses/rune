import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/material_symbols_icons.dart';
import 'package:very_good_infinite_list/very_good_infinite_list.dart';

import '../../utils/l10n.dart';
import '../../utils/rune_log.dart';
import '../../utils/api/list_logs.dart';
import '../../utils/router/navigation.dart';
import '../../utils/settings_page_padding.dart';
import '../../widgets/unavailable_page_on_band.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../messages/all.dart';
import '../settings_library/widgets/settings_button.dart';

class SettingsLogPage extends StatefulWidget {
  const SettingsLogPage({super.key});

  @override
  SettingsLogPageState createState() => SettingsLogPageState();
}

class SettingsLogPageState extends State<SettingsLogPage> {
  final List<LogDetail> _logs = [];
  bool _isLoading = false;
  int _cursor = 0;
  final int _pageSize = 20;

  Future<void> _fetchLogs() async {
    if (_isLoading) return;
    setState(() {
      _isLoading = true;
    });

    try {
      final newLogs = await listLogs(_cursor, _pageSize);
      if (newLogs.isNotEmpty) {
        setState(() {
          _cursor += newLogs.length;
          _logs.addAll(newLogs);
        });
      }
    } catch (e) {
      error$('Error fetching logs: $e');
    } finally {
      setState(() {
        _isLoading = false;
      });
    }
  }

  IconData _getLogLevelIcon(String level) {
    switch (level.toLowerCase()) {
      case 'error':
        return Symbols.error;
      case 'warning':
        return Symbols.warning;
      case 'info':
        return Symbols.info;
      default:
        return Symbols.description;
    }
  }

  void _showLogDetails(LogDetail log) {
    final initialIndex = _logs.indexOf(log);
    $showModal<bool>(
      context,
      (context, $close) => LogDetailDialog(
        logs: _logs,
        initialIndex: initialIndex,
        onClose: () => $close(false),
      ),
      dismissWithEsc: true,
    );
  }

  @override
  Widget build(BuildContext context) {
    return PageContentFrame(
      child: UnavailablePageOnBand(
        child: SettingsPagePadding(
          child: InfiniteList(
            padding: getScrollContainerPadding(context),
            itemCount: _logs.length,
            isLoading: _isLoading,
            onFetchData: _fetchLogs,
            itemBuilder: (context, index) {
              final log = _logs[index];
              return SettingsButton(
                icon: _getLogLevelIcon(log.level),
                title: log.domain,
                subtitle:
                    DateTime.fromMillisecondsSinceEpoch(log.date.toInt() * 1000)
                        .toString(),
                onPressed: () {
                  _showLogDetails(log);
                },
              );
            },
            loadingBuilder: (context) => const Center(child: ProgressRing()),
            emptyBuilder: (context) =>
                const Center(child: Text('No logs available.')),
          ),
        ),
      ),
    );
  }
}

class LogDetailDialog extends StatefulWidget {
  const LogDetailDialog({
    super.key,
    required this.logs,
    required this.initialIndex,
    required this.onClose,
  });

  final List<LogDetail> logs;
  final int initialIndex;
  final VoidCallback onClose;

  @override
  State<LogDetailDialog> createState() => _LogDetailDialogState();
}

class _LogDetailDialogState extends State<LogDetailDialog> {
  late int currentIndex;

  @override
  void initState() {
    super.initState();
    currentIndex = widget.initialIndex;
  }

  void _navigateTo(int index) {
    if (index >= 0 && index < widget.logs.length) {
      setState(() {
        currentIndex = index;
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final log = widget.logs[currentIndex];

    return ContentDialog(
      title: Text(log.level),
      constraints: const BoxConstraints(maxHeight: 320, maxWidth: 520),
      content: Column(
        key: ValueKey(currentIndex),
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(log.domain),
          const SizedBox(height: 4),
          Text(
            DateTime.fromMillisecondsSinceEpoch(log.date.toInt() * 1000)
                .toString(),
          ),
          const SizedBox(height: 8),
          Expanded(
            child: SingleChildScrollView(
              child: SelectableText(
                log.detail,
                style: const TextStyle(height: 1.25),
              ),
            ),
          ),
        ],
      ),
      actions: [
        Row(
          mainAxisAlignment: MainAxisAlignment.spaceBetween,
          children: [
            Row(
              children: [
                Button(
                  onPressed: currentIndex > 0
                      ? () => _navigateTo(currentIndex - 1)
                      : null,
                  child: const Row(
                    children: [
                      Icon(Symbols.arrow_back),
                      SizedBox(width: 4),
                      Text('Previous'),
                    ],
                  ),
                ),
                const SizedBox(width: 8),
                Button(
                  onPressed: currentIndex < widget.logs.length - 1
                      ? () => _navigateTo(currentIndex + 1)
                      : null,
                  child: const Row(
                    children: [
                      Text('Next'),
                      SizedBox(width: 4),
                      Icon(Symbols.arrow_forward),
                    ],
                  ),
                ),
              ],
            ),
            FilledButton(
              onPressed: widget.onClose,
              child: Text(S.of(context).close),
            ),
          ],
        ),
      ],
    );
  }
}
