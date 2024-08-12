import 'package:fluent_ui/fluent_ui.dart';
import 'package:go_router/go_router.dart';

List<NavigationPaneItem> attachNavigationEvent(
    BuildContext context, List<PaneItem> configuration) {
  PaneItem buildPaneItem(PaneItem item) {
    return PaneItem(
      key: item.key,
      icon: item.icon,
      title: item.title,
      body: item.body,
      onTap: () {
        final path = (item.key as ValueKey).value;
        if (GoRouterState.of(context).uri.toString() != path) {
          context.push(path);
        }
        item.onTap?.call();
      },
    );
  }

  return configuration.map<NavigationPaneItem>((e) {
    if (e is PaneItemExpander) {
      return PaneItemExpander(
        key: e.key,
        icon: e.icon,
        title: e.title,
        body: e.body,
        items: e.items.map((item) {
          if (item is PaneItem) return buildPaneItem(item);
          return item;
        }).toList(),
      );
    }
    return buildPaneItem(e);
  }).toList();
}
