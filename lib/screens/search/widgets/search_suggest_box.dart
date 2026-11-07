import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../utils/settings_manager.dart';
import '../../../widgets/no_shortcuts.dart';
import '../../../widgets/rune_clickable.dart';
import '../../../widgets/navigation_bar/page_content_frame.dart';
import '../../../bindings/bindings.dart';
import '../../../providers/responsive_providers.dart';

final SettingsManager settingsManager = SettingsManager();

class SearchSuggestBox extends StatefulWidget {
  final DeviceType deviceType;
  final TextEditingController controller;
  final SearchForResponse? searchResults;
  final void Function() registerSearchTask;

  const SearchSuggestBox({
    super.key,
    required this.deviceType,
    required this.controller,
    required this.searchResults,
    required this.registerSearchTask,
  });

  @override
  SearchSuggestBoxState createState() => SearchSuggestBoxState();
}

class SearchSuggestBoxState extends State<SearchSuggestBox> {
  final searchKey = GlobalKey(debugLabel: 'Search Bar Key');
  final searchFocusNode = FocusNode();

  Timer? _saveDebounce;

  List<String> suggestions = [];

  @override
  void initState() {
    super.initState();
    searchFocusNode.requestFocus();

    widget.controller.addListener(_onControllerChange);
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    // Load suggestions from storage
    settingsManager.getValue<List<dynamic>?>('search_suggestions').then((x) {
      if (x != null && mounted) {
        setState(() {
          suggestions = List<String>.from(x);
        });
      }
    });
  }

  @override
  void dispose() {
    super.dispose();
    searchFocusNode.dispose();
    _saveDebounce?.cancel();
    widget.controller.removeListener(_onControllerChange);
  }

  void _onControllerChange() {
    if (_saveDebounce?.isActive ?? false) _saveDebounce!.cancel();
    _saveDebounce = Timer(const Duration(seconds: 2), () {
      _saveQuery(widget.controller.text);
    });
  }

  void _saveQuery(String query) {
    if (widget.searchResults != null &&
        query.isNotEmpty &&
        !suggestions.contains(query)) {
      suggestions.add(query);
      if (suggestions.length > 64) {
        suggestions.removeAt(0); // Ensure we only keep the latest 64 queries
      }
      settingsManager.setValue('search_suggestions', suggestions);
    }
  }

  /// The popup's height so that it ends above the on-screen keyboard. Fluent
  /// only keeps it clear of the safe area, so the suggestions would otherwise
  /// go behind the keyboard. Most layouts have the box inside a page's frame,
  /// which takes the keyboard out of the view insets.
  double _popupHeight(BuildContext context) {
    final keyboard = PageContentFrame.keyboardOf(context);
    final box = searchKey.currentContext?.findRenderObject();
    if (keyboard == 0 || box is! RenderBox || !box.attached || !box.hasSize) {
      return kAutoSuggestBoxPopupMaxHeight;
    }
    final overlay = Overlay.of(context, rootOverlay: true).context;
    final bottom = box
        .localToGlobal(
          Offset(0, box.size.height),
          ancestor: overlay.findRenderObject(),
        )
        .dy;
    return (MediaQuery.sizeOf(context).height - keyboard - bottom).clamp(
      0.0,
      kAutoSuggestBoxPopupMaxHeight,
    );
  }

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);

    final icon = IgnorePointer(
      child: RuneClickable(
        onPressed: () {},
        child: const Icon(
          Symbols.search,
          size: 16,
        ),
      ),
    );

    return NoShortcuts(
      AutoSuggestBox<String>(
        key: searchKey,
        focusNode: searchFocusNode,
        controller: widget.controller,
        maxPopupHeight: _popupHeight(context),
        unfocusedColor: Colors.transparent,
        style: widget.deviceType == DeviceType.dock
            ? theme.typography.caption
            : null,
        items: suggestions.map((suggestion) {
          return AutoSuggestBoxItem<String>(
            value: suggestion,
            label: suggestion,
            onSelected: () {
              widget.controller.text = suggestion;
              searchFocusNode.unfocus();
              widget.registerSearchTask();
            },
          );
        }).toList(),
        clearButtonEnabled: widget.deviceType != DeviceType.tablet &&
            widget.deviceType != DeviceType.dock &&
            widget.deviceType != DeviceType.band,
        leadingIcon: widget.deviceType == DeviceType.tablet ? icon : null,
        trailingIcon: widget.deviceType == DeviceType.tablet ||
                widget.deviceType == DeviceType.dock ||
                widget.deviceType == DeviceType.band
            ? null
            : icon,
      ),
    );
  }
}
