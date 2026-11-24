import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/utils/hosted_player/hosted_player.dart';

class _RecordingHost implements HostedPlayerHost {
  final commands = <Map<String, dynamic>>[];

  @override
  void handle(
    Map<String, dynamic> command,
    void Function(Map<String, dynamic>) emit,
  ) {
    commands.add(command);
    emit({
      'type': 'hostReady',
      'host': 'other',
      'authorized': true,
      'canPlay': true,
    });
  }
}

class _ReportingHost extends _RecordingHost implements SelfReportingHost {
  var attached = 0;

  @override
  void attach(void Function(Map<String, dynamic>) emit) {
    attached++;
    emit({
      'type': 'hostReady',
      'host': 'other',
      'authorized': false,
      'canPlay': true,
    });
  }
}

/// Reports nothing, so that nothing goes to Rust through the bindings.
class _QuietHost implements SelfReportingHost {
  final commands = <Map<String, dynamic>>[];
  var attached = 0;

  @override
  void attach(void Function(Map<String, dynamic>) emit) => attached++;

  @override
  void handle(
    Map<String, dynamic> command,
    void Function(Map<String, dynamic>) emit,
  ) => commands.add(command);
}

void main() {
  late StreamController<String> commands;
  late List<String> sent;

  setUp(() {
    commands = StreamController<String>();
    sent = [];
  });

  // A stream nobody listened to never finishes closing.
  tearDown(() => unawaited(commands.close()));

  String command(String host, String action) =>
      jsonEncode({'host': host, 'revision': 1, 'action': action});

  test(
    'a host without an implementation answers once that it cannot play',
    () async {
      final bridge = HostedPlayerBridge(
        commands: commands.stream,
        send: sent.add,
      );
      commands.add(command('host', 'set'));
      commands.add(command('host', 'start'));
      commands.add(command('other', 'set'));
      await pumpEventQueue();

      expect(sent.map(jsonDecode), [
        {
          'type': 'hostReady',
          'host': 'host',
          'authorized': true,
          'canPlay': false,
        },
        {
          'type': 'hostReady',
          'host': 'other',
          'authorized': true,
          'canPlay': false,
        },
      ]);
      bridge.dispose();
    },
  );

  test('a self-reporting host is attached as the bridge starts', () async {
    final host = _ReportingHost();
    final bridge = HostedPlayerBridge(
      commands: commands.stream,
      send: sent.add,
      hosts: {'other': host},
    );
    expect(host.attached, 1);
    expect(sent.map(jsonDecode), [
      {
        'type': 'hostReady',
        'host': 'other',
        'authorized': false,
        'canPlay': true,
      },
    ]);

    commands.add(command('other', 'set'));
    await pumpEventQueue();
    expect(host.attached, 1);
    expect(host.commands.single['action'], 'set');
    bridge.dispose();
  });

  test('a registered host gets the command and speaks for itself', () async {
    final host = _RecordingHost();
    final bridge = HostedPlayerBridge(
      commands: commands.stream,
      send: sent.add,
      hosts: {'other': host},
    );
    commands.add(command('other', 'set'));
    commands.add('not json');
    await pumpEventQueue();

    expect(host.commands.single['action'], 'set');
    expect(sent.map(jsonDecode), [
      {
        'type': 'hostReady',
        'host': 'other',
        'authorized': true,
        'canPlay': true,
      },
    ]);
    bridge.dispose();
  });

  test('commands after dispose are ignored', () async {
    final host = _QuietHost();
    final bridge = HostedPlayerBridge(
      commands: commands.stream,
      send: sent.add,
      hosts: {'other': host},
    );
    bridge.dispose();
    commands.add(command('other', 'set'));
    commands.add(command('host', 'set'));
    await pumpEventQueue();

    expect(host.commands, isEmpty);
    expect(sent, isEmpty);
  });

  test('attached() takes the platform hosts and the hub\'s commands', () async {
    final host = _QuietHost();
    platformHosts['other'] = host;
    addTearDown(() => platformHosts.remove('other'));
    final bridge = HostedPlayerBridge.attached();
    expect(host.attached, 1);

    assignRustSignal['HostedPlayerCommand']!(
      HostedPlayerCommand(payload: command('other', 'set')).bincodeSerialize(),
      Uint8List(0),
    );
    await pumpEventQueue();
    expect(host.commands.single['action'], 'set');
    bridge.dispose();
  });
}
