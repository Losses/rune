import 'dart:async';

import 'package:flutter_test/flutter_test.dart';

import 'package:rune/utils/chip_input/search_task.dart';

// Longer than the debounce of the search task.
const _settle = Duration(milliseconds: 400);

void main() {
  test('text typed while a search runs is searched once it is done', () async {
    final pending = <String, Completer<List<String>>>{};
    final task = SearchTask<String, String>(
      notifyWhenStateChange: false,
      searchDelegate: (query) {
        pending[query] = Completer();
        return pending[query]!.future;
      },
    );

    task.search('be');
    await Future.delayed(_settle);
    expect(pending.keys, ['be']);

    // Typed while `be` is still being searched: its debounce is skipped.
    task.search('beat');
    await Future.delayed(_settle);
    expect(pending.keys, ['be']);

    pending['be']!.complete(['Bee Gees']);
    await Future.delayed(Duration.zero);
    expect(pending.keys, ['be', 'beat']);
    expect(task.searchResults, isEmpty, reason: 'results for older text');

    pending['beat']!.complete(['Beatles']);
    await Future.delayed(Duration.zero);
    expect(task.searchResults, ['Beatles']);
    task.dispose();
  });

  test('a search that finishes after dispose does nothing', () async {
    final pending = Completer<List<String>>();
    final task = SearchTask<String, String>(
      notifyWhenStateChange: false,
      searchDelegate: (_) => pending.future,
    );
    task.search('be');
    await Future.delayed(_settle);
    task.dispose();
    pending.complete(['Bee Gees']);
    await Future.delayed(Duration.zero);
  });
}
