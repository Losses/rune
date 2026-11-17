import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

String _githubAnchor(String heading) => heading
    .toLowerCase()
    .replaceAll(RegExp(r'[^\w\- ]'), '')
    .replaceAll(' ', '-');

void main() {
  test('SUPPORT.md official channel link targets an existing heading', () {
    final support = File('SUPPORT.md').readAsStringSync();
    final matches = RegExp(
      r'\[official channel\]\(([^)\s]+)\)',
    ).allMatches(support).toList();

    expect(
      matches,
      hasLength(1),
      reason: 'SUPPORT.md must contain exactly one official channel link',
    );

    final uri = Uri.parse(matches.single.group(1)!);
    expect(uri.isAbsolute, isFalse);
    expect(uri.path, isNotEmpty);
    expect(uri.fragment, isNotEmpty);

    final target = File(uri.path);
    expect(
      target.existsSync(),
      isTrue,
      reason: 'official channel link target ${uri.path} does not exist',
    );

    final anchors = target
        .readAsLinesSync()
        .where((line) => line.startsWith('#'))
        .map(
          (line) => line
              .trim()
              .replaceFirst(RegExp(r'^#{1,6}\s*'), '')
              .replaceFirst(RegExp(r'\s+#+\s*$'), '')
              .trim(),
        )
        .where((heading) => heading.isNotEmpty)
        .map(_githubAnchor);

    expect(
      anchors,
      contains(uri.fragment),
      reason: 'no heading in ${uri.path} produces #${uri.fragment}',
    );
  });
}
