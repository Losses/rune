import 'dart:async';
import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:rune/utils/hosted_player/apple_music_host.dart';
import 'package:rune/utils/hosted_player/hosted_player.dart';

final _ready = jsonEncode({
  'type': 'hostReady',
  'host': 'appleMusic',
  'authorized': true,
  'canPlay': true,
});

const _unavailable = {
  'type': 'hostReady',
  'host': 'appleMusic',
  'authorized': true,
  'canPlay': false,
};

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  group('registerOnPlatform', () {
    const channel = MethodChannel(AppleMusicHost.channelName);
    final messenger =
        TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
    late List<String> asked;

    setUp(() => asked = []);

    tearDown(() {
      platformHosts.remove('appleMusic');
      debugDefaultTargetPlatformOverride = null;
      messenger.setMockMethodCallHandler(channel, null);
    });

    /// A runner that answers `available`, as the Android runner of a build
    /// that includes Apple Music does.
    void runnerWithHost() =>
        messenger.setMockMethodCallHandler(channel, (call) async {
          asked.add(call.method);
          return call.method == 'available' ? true : null;
        });

    for (final platform in [TargetPlatform.macOS, TargetPlatform.iOS]) {
      test('registers the host on ${platform.name} without asking', () async {
        debugDefaultTargetPlatformOverride = platform;
        runnerWithHost();
        await AppleMusicHost.registerOnPlatform();
        expect(platformHosts['appleMusic'], isA<AppleMusicHost>());
        expect(asked, isEmpty);
      });
    }

    test('registers the host on Android when the runner has it', () async {
      debugDefaultTargetPlatformOverride = TargetPlatform.android;
      runnerWithHost();
      await AppleMusicHost.registerOnPlatform();
      expect(platformHosts['appleMusic'], isA<AppleMusicHost>());
      expect(asked, ['available']);
    });

    test(
      'registers no host on Android when nothing handles the channel',
      () async {
        debugDefaultTargetPlatformOverride = TargetPlatform.android;
        await AppleMusicHost.registerOnPlatform();
        expect(platformHosts.containsKey('appleMusic'), isFalse);
      },
    );

    test(
      'registers no host on Android when the runner fails to answer',
      () async {
        debugDefaultTargetPlatformOverride = TargetPlatform.android;
        messenger.setMockMethodCallHandler(
          channel,
          (call) async => throw PlatformException(code: 'broken'),
        );
        await AppleMusicHost.registerOnPlatform();
        expect(platformHosts.containsKey('appleMusic'), isFalse);
      },
    );

    for (final platform in [TargetPlatform.windows, TargetPlatform.linux]) {
      test('registers no host on ${platform.name}', () async {
        debugDefaultTargetPlatformOverride = platform;
        runnerWithHost();
        await AppleMusicHost.registerOnPlatform();
        expect(platformHosts.containsKey('appleMusic'), isFalse);
        expect(asked, isEmpty);
      });
    }
  });

  late MethodChannel channel;
  late AppleMusicHost host;
  late List<MethodCall> calls;
  late List<Map<String, dynamic>> events;
  Object? Function(MethodCall)? answer;
  Future<Object?> Function()? statusAnswer;

  setUp(() {
    channel = const MethodChannel(AppleMusicHost.channelName);
    host = AppleMusicHost(channel: channel);
    calls = [];
    events = [];
    answer = null;
    statusAnswer = null;
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(channel, (call) async {
          calls.add(call);
          if (call.method == 'status') {
            return statusAnswer != null ? statusAnswer!() : _ready;
          }
          return answer?.call(call);
        });
  });

  tearDown(() {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(channel, null);
  });

  Future<void> fromRunner(Object? payload) => TestDefaultBinaryMessengerBinding
      .instance
      .defaultBinaryMessenger
      .handlePlatformMessage(
        channel.name,
        channel.codec.encodeMethodCall(MethodCall('event', payload)),
        (_) {},
      );

  Map<String, dynamic> batch(int revision) => {
    'host': 'appleMusic',
    'revision': revision,
    'actions': [
      {'action': 'start'},
    ],
  };

  test('announces the host, then forwards a batch as it is', () async {
    final batch = {
      'host': 'appleMusic',
      'revision': 12,
      'actions': [
        {
          'action': 'set',
          'ids': ['i.abc'],
        },
        {'action': 'start'},
      ],
    };
    host.handle(batch, events.add);
    await pumpEventQueue();

    expect(calls.map((call) => call.method), ['status', 'batch']);
    expect(jsonDecode(calls.last.arguments as String), batch);
    expect(events.single['type'], 'hostReady');
    expect(events.single['canPlay'], true);

    host.handle(batch, events.add);
    await pumpEventQueue();
    expect(calls.map((call) => call.method), ['status', 'batch', 'batch']);
  });

  test('attaching announces the host once, before any batch', () async {
    host.attach(events.add);
    await pumpEventQueue();
    expect(calls.map((call) => call.method), ['status']);
    expect(events.single['type'], 'hostReady');

    final batch = {
      'host': 'appleMusic',
      'revision': 1,
      'actions': [
        {'action': 'start'},
      ],
    };
    host.handle(batch, events.add);
    await pumpEventQueue();
    expect(calls.map((call) => call.method), ['status', 'batch']);
    expect(jsonDecode(calls.last.arguments as String), batch);
    expect(events, hasLength(1));
  });

  test('batches wait for the announcement made on attach', () async {
    final reply = Completer<Object?>();
    statusAnswer = () => reply.future;
    host.attach(events.add);
    host.handle({
      'host': 'appleMusic',
      'revision': 1,
      'actions': [],
    }, events.add);
    await pumpEventQueue();
    expect(calls.map((call) => call.method), ['status']);

    reply.complete(_ready);
    await pumpEventQueue();
    expect(calls.map((call) => call.method), ['status', 'batch']);
    expect(events.map((event) => event['type']), ['hostReady']);
  });

  test('network states from the runner reach the bridge as they are', () async {
    host.attach(events.add);
    await pumpEventQueue();
    events.clear();

    const offline = {
      'type': 'networkState',
      'host': 'appleMusic',
      'online': false,
      'downloaded': ['i.a', 'i.b'],
    };
    const online = {
      'type': 'networkState',
      'host': 'appleMusic',
      'online': true,
    };
    await fromRunner(jsonEncode(offline));
    await fromRunner(jsonEncode(online));
    expect(events, [offline, online]);
  });

  test(
    'the network coming back is told once, and only after it was gone',
    () async {
      host.attach(events.add);
      await pumpEventQueue();
      var back = 0;
      final subscription = host.networkBack.listen((_) => back++);
      addTearDown(subscription.cancel);
      Future<void> report(bool online) => fromRunner(
        jsonEncode({
          'type': 'networkState',
          'host': 'appleMusic',
          'online': online,
        }),
      );

      // A host that starts with a network has none coming back.
      await report(true);
      await pumpEventQueue();
      expect(back, 0);

      await report(false);
      await report(false);
      await report(true);
      await report(true);
      await pumpEventQueue();
      expect(back, 1);
    },
  );

  test('events from the runner reach the bridge once attached', () async {
    host.attach(events.add);
    await pumpEventQueue();
    events.clear();

    const status = {
      'type': 'jukeboxStatus',
      'host': 'appleMusic',
      'revision': 1,
      'currentIndex': 0,
      'playing': true,
      'positionMs': 900,
    };
    const ready = {
      'type': 'hostReady',
      'host': 'appleMusic',
      'authorized': false,
      'canPlay': false,
    };
    await fromRunner(jsonEncode(status));
    await fromRunner(jsonEncode(ready));
    expect(events, [status, ready]);
  });

  test('authorize passes interactive and parses tokens', () async {
    answer = (call) => {
      'authorized': true,
      'token': 'user',
      'developerToken': 'dev',
    };
    final authorization = await host.authorize(interactive: true);
    expect(calls.single.arguments, {'interactive': true});
    expect(
      authorization,
      isA<HostAuthorization>()
          .having((value) => value.token, 'token', 'user')
          .having((value) => value.developerToken, 'developerToken', 'dev'),
    );
  });

  test('authorize without tokens or authorization says why', () {
    HostAuthorizationFailure? failure(Object? reply) =>
        switch (AppleMusicHost.parseAuthorization(reply)) {
          HostAuthorizationFailed(:final failure) => failure,
          HostAuthorization() => null,
        };

    expect(
      failure({'authorized': false, 'reason': 'refused'}),
      HostAuthorizationFailure.refused,
    );
    expect(failure({'authorized': false}), HostAuthorizationFailure.refused);
    expect(
      failure({'authorized': false, 'reason': 'unsupported'}),
      HostAuthorizationFailure.unsupported,
    );
    expect(
      failure({'authorized': true, 'reason': 'noToken'}),
      HostAuthorizationFailure.noToken,
    );
    expect(failure({'authorized': true}), HostAuthorizationFailure.noToken);
    expect(
      failure({'authorized': true, 'reason': 'account'}),
      HostAuthorizationFailure.account,
    );
    expect(
      failure({'authorized': true, 'reason': 'privacy'}),
      HostAuthorizationFailure.privacy,
    );
    expect(
      failure({'authorized': true, 'reason': 'somethingNew'}),
      HostAuthorizationFailure.noToken,
    );
    expect(failure(null), HostAuthorizationFailure.unavailable);
    expect(failure('{}'), HostAuthorizationFailure.unavailable);
    final noDeveloper = AppleMusicHost.parseAuthorization({
      'authorized': true,
      'token': 'user',
    });
    expect(
      noDeveloper,
      isA<HostAuthorization>()
          .having((value) => value.token, 'token', 'user')
          .having((value) => value.developerToken, 'developerToken', isNull),
    );
  });

  test('batches sent before the status reply wait for it, in order', () async {
    final reply = Completer<Object?>();
    statusAnswer = () => reply.future;
    host.handle(batch(1), events.add);
    host.handle(batch(2), events.add);
    await pumpEventQueue();
    expect(calls.map((call) => call.method), ['status']);
    expect(events, isEmpty);

    reply.complete(_ready);
    await pumpEventQueue();
    expect(events.map((event) => event['type']), ['hostReady']);
    expect(calls.map((call) => call.method), ['status', 'batch', 'batch']);
    expect(calls.skip(1).map((call) => jsonDecode(call.arguments as String)), [
      batch(1),
      batch(2),
    ]);
  });

  final failures = <String, Future<Object?> Function()>{
    'PlatformException': () async => throw PlatformException(code: 'x'),
    'MissingPluginException': () async => throw MissingPluginException(),
    'another error': () async => throw StateError('boom'),
    'null': () async => null,
    'a reply that is not text': () async => 42,
    'text that is not an object': () async => '[1]',
    'text that does not parse': () async => '{',
  };
  for (final entry in failures.entries) {
    test('status failing with ${entry.key} settles on unavailable', () async {
      statusAnswer = entry.value;
      host.handle(batch(1), events.add);
      host.handle(batch(2), events.add);
      await pumpEventQueue();
      // The announcement, then once more for each batch, which goes no further.
      expect(events, [_unavailable, _unavailable, _unavailable]);
      expect(calls.map((call) => call.method), ['status']);
    });
  }

  test(
    'a batch after a failed announcement on attach hears it again',
    () async {
      statusAnswer = () async => throw MissingPluginException();
      host.attach(events.add);
      await pumpEventQueue();
      expect(events, [_unavailable]);

      host.handle(batch(1), events.add);
      await pumpEventQueue();
      expect(events, [_unavailable, _unavailable]);
      expect(calls.map((call) => call.method), ['status']);
    },
  );

  test(
    'a host that says it is ready after a failed announcement plays again',
    () async {
      statusAnswer = () async => throw PlatformException(code: 'x');
      host.attach(events.add);
      host.handle(batch(1), events.add);
      await pumpEventQueue();
      expect(events, [_unavailable, _unavailable]);

      // The runner's own announcement replaces the one made for it.
      await fromRunner(_ready);
      host.handle(batch(2), events.add);
      await pumpEventQueue();
      expect(events, [_unavailable, _unavailable, jsonDecode(_ready)]);
      expect(calls.map((call) => call.method), ['status', 'batch']);
      expect(jsonDecode(calls.last.arguments as String), batch(2));
    },
  );

  test('a host that cannot play holds batches until it says it can', () async {
    final revoked = {
      'type': 'hostReady',
      'host': 'appleMusic',
      'authorized': false,
      'canPlay': false,
    };
    statusAnswer = () async => jsonEncode(revoked);
    host.attach(events.add);
    host.handle(batch(1), events.add);
    await pumpEventQueue();
    expect(events, [revoked, revoked]);
    expect(calls.map((call) => call.method), ['status']);

    // Access granted later; a positive answer is never repeated.
    await fromRunner(_ready);
    host.handle(batch(2), events.add);
    await pumpEventQueue();
    expect(events, [revoked, revoked, jsonDecode(_ready)]);
    expect(calls.map((call) => call.method), ['status', 'batch']);
    expect(jsonDecode(calls.last.arguments as String), batch(2));
  });

  test('authorize failing is unavailable', () async {
    final unavailable = isA<HostAuthorizationFailed>().having(
      (value) => value.failure,
      'failure',
      HostAuthorizationFailure.unavailable,
    );
    answer = (call) => throw PlatformException(code: 'denied');
    expect(await host.authorize(interactive: true), unavailable);
    answer = (call) => throw StateError('boom');
    expect(await host.authorize(), unavailable);
  });

  test('events that do not parse are ignored', () async {
    host.handle({
      'host': 'appleMusic',
      'revision': 1,
      'actions': [],
    }, events.add);
    await pumpEventQueue();
    events.clear();
    for (final payload in <Object?>['{', '[1]', 7, null]) {
      await fromRunner(payload);
    }
    expect(events, isEmpty);
  });

  group('access lost', () {
    Map<String, dynamic> ready({required bool authorized}) => {
      'type': 'hostReady',
      'host': 'appleMusic',
      'authorized': authorized,
      'canPlay': authorized,
    };

    test('a host that starts without access loses nothing', () async {
      var lost = 0;
      host.accessLost.listen((_) => lost++);
      statusAnswer = () async => jsonEncode(ready(authorized: false));
      host.attach(events.add);
      await pumpEventQueue();
      await fromRunner(jsonEncode(ready(authorized: false)));
      await pumpEventQueue();

      expect(lost, 0);
      expect(events, hasLength(2));
    });

    test('access withdrawn while Rune runs is lost once', () async {
      var lost = 0;
      host.accessLost.listen((_) => lost++);
      host.attach(events.add);
      await pumpEventQueue();
      await fromRunner(jsonEncode(ready(authorized: false)));
      await fromRunner(jsonEncode(ready(authorized: false)));
      await pumpEventQueue();
      expect(lost, 1);

      // Granted again, then withdrawn again.
      await fromRunner(jsonEncode(ready(authorized: true)));
      await fromRunner(jsonEncode(ready(authorized: false)));
      await pumpEventQueue();
      expect(lost, 2);
      // The hub still hears every event.
      expect(events, hasLength(5));
    });

    test(
      'a track the host may not play loses access while access is not reported',
      () async {
        var lost = 0;
        host.accessLost.listen((_) => lost++);
        statusAnswer = () async => jsonEncode(ready(authorized: false));
        host.attach(events.add);
        await pumpEventQueue();
        for (final reason in ['network', 'unplayable', 'notAuthorized']) {
          await fromRunner(
            jsonEncode({
              'type': 'jukeboxItemFailed',
              'host': 'appleMusic',
              'revision': 1,
              'index': 0,
              'reason': reason,
            }),
          );
        }
        await pumpEventQueue();

        expect(lost, 1);
        expect(events, hasLength(4));
      },
    );

    test(
      'a track that fails while MusicKit reports access loses nothing',
      () async {
        var lost = 0;
        host.accessLost.listen((_) => lost++);
        // The status reports access.
        host.attach(events.add);
        await pumpEventQueue();
        await fromRunner(
          jsonEncode({
            'type': 'jukeboxItemFailed',
            'host': 'appleMusic',
            'revision': 1,
            'index': 0,
            'reason': 'notAuthorized',
          }),
        );
        await pumpEventQueue();

        expect(lost, 0);
        // The hub still hears it, and shows the track as revoked.
        expect(events.last['type'], 'jukeboxItemFailed');
      },
    );

    test(
      'tracks of one batch failing one after another lose access once',
      () async {
        var lost = 0;
        host.accessLost.listen((_) => lost++);
        // Whether the host has access is not known.
        statusAnswer = () async => throw MissingPluginException();
        host.attach(events.add);
        await pumpEventQueue();
        Future<void> failed(int revision, int index) => fromRunner(
          jsonEncode({
            'type': 'jukeboxItemFailed',
            'host': 'appleMusic',
            'revision': revision,
            'index': index,
            'reason': 'notAuthorized',
          }),
        );
        await failed(4, 0);
        await failed(4, 1);
        await failed(4, 2);
        await pumpEventQueue();
        expect(lost, 1);

        await failed(5, 0);
        await pumpEventQueue();
        expect(lost, 2);
      },
    );

    test('a status that failed is not taken for access', () async {
      var lost = 0;
      host.accessLost.listen((_) => lost++);
      statusAnswer = () async => throw MissingPluginException();
      host.attach(events.add);
      await pumpEventQueue();
      // The hub hears a host that cannot play, which says nothing of access.
      expect(events, [_unavailable]);

      await fromRunner(jsonEncode(ready(authorized: false)));
      await pumpEventQueue();
      expect(lost, 0);
    });
  });
}
