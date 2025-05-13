import 'query_list.dart';

int? getPlaylistIdFromQueryList(QueryList x) {
  if (x.length != 1) {
    return null;
  }

  final firstQuery = x[0];

  if (firstQuery.$1 != 'lib::playlist') {
    return null;
  }

  return int.tryParse(firstQuery.$2);
}
