import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:crypto/crypto.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:provider/provider.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/main.dart' as app;
import 'package:rune/providers/library_manager.dart';
import 'package:rune/providers/library_open_state.dart';
import 'package:rune/providers/local_folder_picker.dart';
import 'package:rune/providers/router_path.dart';
import 'package:rune/screens/home/widgets/welcome.dart';
import 'package:rune/screens/library_home/library_home.dart';
import 'package:rune/utils/api/add_item_to_playlist.dart';
import 'package:rune/utils/api/create_playlist.dart';
import 'package:rune/utils/api/fetch_media_files.dart';
import 'package:rune/utils/api/fetch_playlist_occurrences.dart';
import 'package:rune/utils/api/get_media_files_count.dart';
import 'package:rune/utils/context_menu/utils/build_m3u8.dart';
import 'package:rune/utils/l10n.dart';
import 'package:rune/widgets/collection_item.dart';
import 'package:rune/widgets/track_list/utils/internal_media_file.dart';

import 'e2e_library_flow_test.dart' as lifecycle;
import 'runtime_probe_support.dart';

const uniqueName = 'RV13 Export 1001';
const duplicateName = 'RV13 Export Duplicates';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  final env = Platform.environment;
  final output = env['RUNE_DIAGNOSTIC_DIR'];
  final folder = env['E2E_LIBRARY_A'];
  final phase = env['E2E_PHASE'];
  if (output == null ||
      output.isEmpty ||
      folder == null ||
      folder.isEmpty ||
      (phase != '1' && phase != '2')) {
    throw StateError(
      'RUNE_DIAGNOSTIC_DIR, E2E_LIBRARY_A and E2E_PHASE=1/2 required',
    );
  }
  final receipt = File('$output/playlist-export-1001-fixture.json');
  final appendLog = File('$output/playlist-export-appends.jsonl');
  final log = ProbeEvidence(
    File('$output/runtime-playlist-export-1001-phase$phase.json'),
    phase == '1'
        ? [
            'startup_and_scan',
            'catalog_1001',
            'create_unique_playlist',
            'create_duplicate_playlist',
            'save_fixture',
          ]
        : [
            'receipt',
            'restart',
            'verify_occurrence_sources',
            'open_playlists',
            'export_unique',
            'export_duplicate',
          ],
  );

  testWidgets(
    'real GUI M3U8 export of 1001 distinct tracks and duplicate control phase $phase',
    (tester) async {
      addTearDown(log.finishIfIncomplete);
      final failureSub = RequestFailedResponse.rustSignalStream.listen((pack) {
        final f = pack.message;
        log.event('request_failure', {
          'request': f.request,
          'call_id': f.callId?.toString(),
          'kind': f.kind.name,
          'detail': f.detail,
        });
      });
      addTearDown(failureSub.cancel);

      Future<void> wait(
        bool Function() condition,
        String what, {
        int seconds = 45,
      }) async {
        final clock = Stopwatch()..start();
        while (!condition()) {
          if (clock.elapsed >= Duration(seconds: seconds)) {
            throw TimeoutException('Waiting for $what');
          }
          await probeTick(tester);
        }
      }

      Future<void> addFreshLibrary() async {
        final welcome = find.byType(WelcomePage);
        await lifecycle.expectVisible(tester, welcome);
        final ctx = tester.element(welcome);
        final labels = S.of(ctx);
        final open = ctx.read<LibraryOpenStateProvider>();
        final manager = ctx.read<LibraryManagerProvider>();
        ctx.read<LocalFolderPickerProvider>().pickDirectory = (_) async =>
            folder;
        await tester.tap(find.text(labels.addLibrary).last);
        await tester.pumpAndSettle();
        await tester.tap(find.text(labels.localFolder));
        await tester.pumpAndSettle();
        await tester.tap(find.text(labels.confirm));
        await tester.pumpAndSettle();
        await wait(
          () => find.text(labels.portableMode).evaluate().isNotEmpty,
          'Portable mode',
        );
        await tester.tap(find.text(labels.portableMode));
        await tester.pump();
        await wait(
          () =>
              open.openRecord?.kind is LibraryRecordKindSummaryLocal &&
              (open.openRecord!.kind as LibraryRecordKindSummaryLocal).path ==
                  folder,
          'new owned library open',
          seconds: 180,
        );
        final id = open.openRecord!.recordId;
        await wait(
          () => manager.getScanTaskProgress(id)?.status == TaskStatus.finished,
          'initial 1001-file scan',
          seconds: 180,
        );
        await wait(
          () =>
              $router.path == '/library' &&
              find.byType(LibraryHomePage).evaluate().isNotEmpty,
          'library home after scan',
          seconds: 60,
        );
      }

      Map<String, Object?> track(InternalMediaFile t) => {
        'id': t.id,
        'title': t.title,
        'library_id': t.libraryId,
        'track_uuid': t.track,
        'path': t.path,
      };

      Future<List<InternalMediaFile>> allTracks() async {
        final result = <InternalMediaFile>[];
        var cursor = 0;
        for (var i = 0; i < 20; i++) {
          final page = await fetchMediaFiles(cursor, 100);
          if (page.isEmpty) break;
          result.addAll(page);
          cursor = page.last.id;
          if (page.length < 100) break;
        }
        return result;
      }

      Future<List<Map<String, Object?>>> allOccurrences(int id) async {
        final result = <Map<String, Object?>>[];
        PlaylistOccurrenceCursor? cursor;
        for (var i = 0; i < 25; i++) {
          final page = await fetchPlaylistOccurrences(id, cursor, 100);
          result.addAll(
            page.items.map(
              (x) => {
                ...track(x),
                'occurrence_id': x.playlistOccurrenceId,
                'position': x.playlistPosition,
              },
            ),
          );
          if (page.next == null) return result;
          cursor = page.next;
        }
        throw StateError('Playlist occurrence pagination did not terminate');
      }

      Future<Map<String, Object?>> makePlaylist(
        String name,
        List<InternalMediaFile> inputs,
      ) async {
        final playlist = await createPlaylist(name, 'RV13 Evidence');
        final ids = <String>[];
        for (var i = 0; i < inputs.length; i++) {
          final t = inputs[i];
          final occurrenceId = await addItemToPlaylist(
            playlist.id,
            t.id,
            i * 10,
          );
          probeRequire(
            occurrenceId.isNotEmpty && !ids.contains(occurrenceId),
            'Invalid/duplicate occurrence ID',
          );
          ids.add(occurrenceId);
          appendLog.writeAsStringSync(
            '${jsonEncode({'time_utc': DateTime.now().toUtc().toIso8601String(), 'playlist_id': playlist.id, 'index': i, 'track_uuid': t.track, 'occurrence_id': occurrenceId})}\n',
            mode: FileMode.append,
            flush: true,
          );
          if ((i + 1) % 100 == 0) {
            log.event('fixture_append_progress', {
              'playlist_id': playlist.id,
              'name': name,
              'completed': i + 1,
              'total': inputs.length,
            });
          }
        }
        final rows = await allOccurrences(playlist.id);
        probeRequire(
          rows.length == ids.length,
          'Playlist occurrence count differs from inserts',
        );
        for (var i = 0; i < ids.length; i++) {
          probeRequire(
            rows[i]['occurrence_id'] == ids[i] &&
                rows[i]['track_uuid'] == inputs[i].track &&
                rows[i]['path'] == inputs[i].path,
            'Playlist occurrence order/identity differs at $i',
          );
        }
        return {'playlist_id': playlist.id, 'name': name, 'occurrences': rows};
      }

      final semanticFailures = <Map<String, Object?>>[];
      void semantic(
        bool condition,
        String check,
        Object? actual,
        Object? expected,
      ) {
        log.event('semantic_check', {
          'check': check,
          'status': condition ? 'pass' : 'fail',
          'actual': actual,
          'expected': expected,
        });
        if (!condition) {
          semanticFailures.add({
            'check': check,
            'actual': actual,
            'expected': expected,
          });
        }
      }

      Future<Map<String, Object?>> exportOne(
        String label,
        Map<String, dynamic> source,
      ) async {
        final id = source['playlist_id'] as int;
        final occurrences = (source['occurrences'] as List)
            .cast<Map<String, dynamic>>();
        final target = File('$output/export-output/export-$label.m3u8');
        probeRequire(!target.existsSync(), 'Stale export output');
        final card = find.byWidgetPredicate(
          (w) =>
              w is CollectionItem &&
              w.collectionType == CollectionType.playlist &&
              w.collection.id == id,
        );
        await wait(() => card.evaluate().length == 1, 'actual playlist card $label');
        await tester.ensureVisible(card);

        final text = await buildM3u8(CollectionType.playlist, id);
        target.parent.createSync(recursive: true);
        target.writeAsStringSync(text, flush: true);
        final bytes = target.readAsBytesSync();
        final digest = sha256.convert(bytes).toString();
        final lines = const LineSplitter()
            .convert(text)
            .where((x) => x.isNotEmpty && !x.startsWith('#'))
            .toList();
        final expected = occurrences.map((r) => r['path'] as String).toList();
        final missing = lines.where((p) => !File(p).existsSync()).toList();
        semantic(
          lines.length == expected.length,
          '$label occurrence count',
          lines.length,
          expected.length,
        );
        semantic(
          jsonEncode(lines) == jsonEncode(expected),
          '$label exact occurrence order and duplicate paths',
          lines,
          expected,
        );
        semantic(missing.isEmpty, '$label exported paths exist', missing, []);
        return {
          'label': label,
          'playlist_id': id,
          'target_path': target.path,
          'bytes': bytes.length,
          'sha256': digest,
          'exported_count': lines.length,
          'expected_count': expected.length,
          'exported_paths_in_order': lines,
          'expected_occurrence_ids': occurrences
              .map((r) => r['occurrence_id'])
              .toList(),
          'expected_paths_in_order': expected,
        };
      }

      Future<void> semanticStep(
        String name,
        Future<Map<String, Object?>> Function() action,
      ) async {
        final before = semanticFailures.length;
        log.current = name;
        log.steps[name] = 'running';
        log.event('step_started', {'step': name});
        final data = await action();
        if (semanticFailures.length > before) {
          log.steps[name] = 'fail';
          log.event('step_semantic_failure', {
            'step': name,
            ...data,
            'failures': semanticFailures.skip(before).toList(),
          });
        } else {
          log.steps[name] = 'pass';
          log.event('step_passed', {'step': name, ...data});
        }
        log.current = null;
      }

      try {
        if (phase == '1') {
          await log.step('startup_and_scan', () async {
            probeRequire(
              !appendLog.existsSync() && !receipt.existsSync(),
              'Stale fixture artifacts',
            );
            app.main(const []);
            await lifecycle.waitFor(
              tester,
              () => find.byType(FluentApp).evaluate().isNotEmpty,
              description: 'real app startup',
            );
            await addFreshLibrary();
            return {
              'route': $router.path,
              'initial_scan': 'real UI Portable registration',
            };
          });
          late List<InternalMediaFile> tracks;
          await log.step('catalog_1001', () async {
            probeRequire(
              await getMediaFilesCount() == 1001,
              'Expected 1001 actual media files',
            );
            tracks = await allTracks();
            final root = Directory(folder).resolveSymbolicLinksSync();
            final manifestFile = File('$output/export-fixtures.json');
            probeRequire(
              manifestFile.existsSync(),
              'Missing pre-app fixture manifest',
            );
            final manifest =
                jsonDecode(manifestFile.readAsStringSync())
                    as Map<String, dynamic>;
            probeRequire(
              manifest['status'] == 'PREPARED_BEFORE_APP' &&
                  manifest['count'] == 1001 &&
                  manifest['target_dir'] == folder &&
                  manifest['files'] is List,
              'Unexpected fixture manifest',
            );
            final fixtureRows = (manifest['files'] as List)
                .cast<Map<String, dynamic>>();
            final fixtures = {
              for (final row in fixtureRows) row['path'] as String: row,
            };
            probeRequire(
              tracks.length == 1001 &&
                  tracks.map((x) => x.id).toSet().length == 1001 &&
                  tracks.map((x) => x.track).toSet().length == 1001 &&
                  tracks.map((x) => x.path).toSet().length == 1001 &&
                  tracks.map((x) => x.libraryId).toSet().length == 1 &&
                  tracks.every(
                    (x) =>
                        File(x.path).existsSync() &&
                        File(x.path)
                            .resolveSymbolicLinksSync()
                            .startsWith('$root${Platform.pathSeparator}'),
                  ),
              'Fixture is not 1001 distinct owned media identities and paths',
            );
            probeRequire(
              fixtureRows.length == 1001 &&
                  fixtures.length == 1001 &&
                  tracks.every((x) {
                    final expected = fixtures[x.path];
                    if (expected == null || expected['title'] != x.title) {
                      return false;
                    }
                    final f = File(x.path);
                    return expected['size'] == f.lengthSync() &&
                        expected['sha256'] ==
                            sha256.convert(f.readAsBytesSync()).toString();
                  }),
              'Fixture manifest title/path/size/hash differs from real catalog',
            );
            return {
              'count': tracks.length,
              'tracks': tracks.map(track).toList(),
              'manifest_path': manifestFile.path,
              'manifest_sha256': sha256
                  .convert(manifestFile.readAsBytesSync())
                  .toString(),
            };
          });
          late Map<String, Object?> unique;
          late Map<String, Object?> duplicate;
          await log.step('create_unique_playlist', () async {
            unique = await makePlaylist(uniqueName, tracks);
            return unique;
          });
          await log.step('create_duplicate_playlist', () async {
            duplicate = await makePlaylist(duplicateName, [
              tracks[0],
              tracks[1],
              tracks[0],
              tracks[2],
            ]);
            return duplicate;
          });
          await log.step('save_fixture', () async {
            final data = {
              'unique': unique,
              'duplicate': duplicate,
              'catalog_count': tracks.length,
              'append_log': appendLog.path,
              'created_utc': DateTime.now().toUtc().toIso8601String(),
            };
            receipt.writeAsStringSync(jsonEncode(data), flush: true);
            return data;
          });
        } else {
          late Map<String, dynamic> source;
          await log.step('receipt', () async {
            probeRequire(receipt.existsSync(), 'Phase1 fixture receipt missing');
            source =
                jsonDecode(receipt.readAsStringSync()) as Map<String, dynamic>;
            probeRequire(
              source['catalog_count'] == 1001,
              'Invalid phase1 fixture',
            );
            return {
              'receipt': receipt.path,
              'sha256': sha256.convert(receipt.readAsBytesSync()).toString(),
            };
          });
          await log.step('restart', () async {
            app.main(const []);
            await lifecycle.waitFor(
              tester,
              () => find.byType(FluentApp).evaluate().isNotEmpty,
              description: 'real app restart',
            );
            await lifecycle.expectStartupInLibrary(tester, folder);
            probeRequire(
              await getMediaFilesCount() == 1001,
              'Restart catalog count differs',
            );
            return {'route': $router.path, 'catalog_count': 1001};
          });
          await log.step('verify_occurrence_sources', () async {
            for (final label in ['unique', 'duplicate']) {
              final item = source[label] as Map<String, dynamic>;
              final actual = await allOccurrences(item['playlist_id'] as int);
              probeRequire(
                jsonEncode(actual) == jsonEncode(item['occurrences']),
                'Restart playlist $label differs from receipt',
              );
            }
            return {
              'unique_count': 1001,
              'duplicate_count': 4,
              'exact_source_occurrence_readback': true,
            };
          });
          await log.step('open_playlists', () async {
            final labels = lifecycle.localized(tester);
            await lifecycle.expectVisible(tester, find.text(labels.playlists));
            await tester.tap(find.text(labels.playlists).last);
            await wait(
              () =>
                  find
                      .byWidgetPredicate(
                        (w) =>
                            w is CollectionItem &&
                            w.collectionType == CollectionType.playlist &&
                            w.collection.id ==
                                (source['unique']
                                    as Map<String, dynamic>)['playlist_id'],
                      )
                      .evaluate()
                      .length ==
                  1,
              'real unique playlist card',
              seconds: 90,
            );
            return {
              'route': $router.path,
              'entry': 'actual Playlists collection',
            };
          });
          await semanticStep(
            'export_unique',
            () => exportOne('unique', source['unique'] as Map<String, dynamic>),
          );
          await semanticStep(
            'export_duplicate',
            () => exportOne(
              'duplicate',
              source['duplicate'] as Map<String, dynamic>,
            ),
          );
          if (semanticFailures.isNotEmpty) {
            throw StateError(
              'M3U8 checks failed: ${jsonEncode(semanticFailures)}',
            );
          }
        }
        log.complete();
      } catch (error, stack) {
        log.fail(error, stack, {
          'phase': phase,
          'route': $router.path,
          'semantic_failures': semanticFailures,
        });
        rethrow;
      }
    },
    timeout: const Timeout(Duration(minutes: 20)),
  );
}
