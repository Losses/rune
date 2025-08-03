import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/material_symbols_icons.dart';
import 'package:very_good_infinite_list/very_good_infinite_list.dart';

import '../../utils/l10n.dart';
import '../../utils/rune_log.dart';
import '../../utils/settings_page_padding.dart';
import '../../utils/api/clear_logs.dart';
import '../../utils/api/remove_log.dart';
import '../../utils/api/list_logs.dart';
import '../../utils/router/navigation.dart';
import '../../utils/router/router_aware_flyout_controller.dart';
import '../../utils/dialogs/remove_dialog_on_band.dart';
import '../../widgets/context_menu_wrapper.dart';
import '../../widgets/no_items.dart';
import '../../widgets/responsive_dialog_actions.dart';
import '../../widgets/unavailable_page_on_band.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../screens/settings_log/widgets/log_detail_dialog.dart';
import '../../screens/settings_log/widgets/log_item.dart';
import '../../bindings/bindings.dart';

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

  final _contextController = RouterAwareFlyoutController();
  final _contextAttachKey = GlobalKey();

  @override
  void initState() {
    super.initState();
    _fetchLogs();
  }

  @override
  dispose() {
    super.dispose();
    _contextController.dispose();
  }

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

  void _showLogDetails(int index) {
    $showModal<bool>(
      context,
      (context, $close) => LogDetailDialog(
        logs: _logs,
        initialIndex: index,
        onClose: () => $close(false),
      ),
      dismissWithEsc: true,
      barrierDismissible: true,
    );
  }

  void _refresh() {
    _cursor = 0;
    _logs.clear();
    _fetchLogs();
  }

  bool _isRemoving = false;

  Future<void> _onConfirmRemove(int id) async {
    setState(() {
      _isRemoving = true;
    });
    await removeLog(id);
    setState(() {
      _isRemoving = false;
    });
  }

  void _removeLog(int index) async {
    final id = _logs[index].id;
    final result = await $showModal<bool>(
      context,
      (context, $close) => RemoveDialogOnBand(
        $close: $close,
        onConfirm: () => $close(true),
        child: ContentDialog(
          title: Column(
            children: [
              SizedBox(height: 8),
              Text(S.of(context).removeLogTitle),
            ],
          ),
          content: Text(
            S.of(context).removeLogSubtitle,
          ),
          actions: [
            ResponsiveDialogActions(
              FilledButton(
                onPressed: _isRemoving ? null : () => $close(true),
                child: Text(S.of(context).delete),
              ),
              Button(
                onPressed: _isRemoving ? null : () => $close(false),
                child: Text(S.of(context).cancel),
              ),
            ),
          ],
        ),
      ),
      dismissWithEsc: true,
      barrierDismissible: true,
    );

    if (result == true) {
      await _onConfirmRemove(id);
      _refresh();
    }
  }

  void _clearLog() async {
    final result = await $showModal<bool>(
      context,
      (context, $close) => RemoveDialogOnBand(
        $close: $close,
        onConfirm: () => $close(true),
        child: ContentDialog(
          title: Column(
            children: [
              SizedBox(height: 8),
              Text(S.of(context).clearLogTitle),
            ],
          ),
          content: Text(
            S.of(context).clearLogSubtitle,
          ),
          actions: [
            ResponsiveDialogActions(
              FilledButton(
                onPressed: _isRemoving ? null : () => $close(true),
                child: Text(S.of(context).delete),
              ),
              Button(
                onPressed: _isRemoving ? null : () => $close(false),
                child: Text(S.of(context).cancel),
              ),
            ),
          ],
        ),
      ),
      dismissWithEsc: true,
      barrierDismissible: true,
    );

    if (result == true) {
      await clearLogs();
      _refresh();
    }
  }

  void _openListContextMenu(
    Offset localPosition,
  ) async {
    final targetContext = _contextAttachKey.currentContext;

    if (targetContext == null) return;
    final box = targetContext.findRenderObject() as RenderBox;
    final position = box.localToGlobal(
      localPosition,
      ancestor: Navigator.of(context).context.findRenderObject(),
    );

    if (!context.mounted) return;

    _contextController.showFlyout(
      position: position,
      builder: (context) => MenuFlyout(
        items: [
          MenuFlyoutItem(
            leading: const Icon(Symbols.refresh),
            text: Text(S.of(context).refresh),
            onPressed: _refresh,
          ),
          MenuFlyoutItem(
            leading: const Icon(Symbols.delete),
            text: Text(S.of(context).deleteAll),
            onPressed: _clearLog,
          ),
        ],
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    return PageContentFrame(
      child: UnavailablePageOnBand(
        child: ContextMenuWrapper(
          contextAttachKey: _contextAttachKey,
          contextController: _contextController,
          onContextMenu: _openListContextMenu,
          onMiddleClick: (_) {},
          child: SettingsPagePadding(
            child: (!_isLoading && _logs.isEmpty)
                ? Padding(
                    padding: getScrollContainerPadding(context),
                    child: Center(
                      child: NoItems(
                        title: S.of(context).noLogsAvailable,
                        hasRecommendation: false,
                        reloadData: _refresh,
                        showDetail: false,
                      ),
                    ),
                  )
                : InfiniteList(
                    padding: getScrollContainerPadding(context),
                    itemCount: _logs.length,
                    isLoading: _isLoading,
                    onFetchData: _fetchLogs,
                    itemBuilder: (context, index) {
                      final log = _logs[index];
                      return SizedBox(
                        height: 72,
                        child: LogItem(
                          index: index,
                          log: log,
                          onTap: _showLogDetails,
                          onRemove: _removeLog,
                        ),
                      );
                    },
                    loadingBuilder: (context) =>
                        const Center(child: ProgressRing()),
                    emptyBuilder: (context) => Container(),
                  ),
          ),
        ),
      ),
    );
  }
}
