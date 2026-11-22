import 'dart:async';

import 'package:flutter_test/flutter_test.dart';

import '../bindings/bindings.dart';
import '../utils/api/request_failure.dart';
import '../utils/recover_failed_startup_open.dart';

RequestFailure _openFailure(OpenLibraryFailureKind kind) => RequestFailure(
  'OpenLibraryRequest',
  RequestErrorKind.unavailable,
  kind.name,
  RequestFailureCauseLibraryOpenFailed(kind: kind),
);

class _Startup {
  _Startup({
    required bool opened,
    this.failure,
    this.signIn = _cancel,
    this.libraryOpen = false,
  }) : libraryOpened = Future.value(opened);

  static Future<bool> _cancel() async => false;

  final Future<bool> libraryOpened;
  final Object? failure;
  final Future<bool> Function() signIn;
  bool libraryOpen;
  var signIns = 0;
  final chooser = <Object?>[];

  Future<void> recover() => recoverFailedStartupOpen(
    libraryOpened: libraryOpened,
    failure: () => failure,
    libraryIsOpen: () => libraryOpen,
    signInAgain: () {
      signIns += 1;
      return signIn();
    },
    showChooser: chooser.add,
  );
}

void main() {
  test('a library that opens at startup stays open', () async {
    final startup = _Startup(opened: true);
    await startup.recover();

    expect(startup.chooser, isEmpty);
    expect(startup.signIns, 0);
  });

  test('waits for the startup open before choosing', () async {
    final opened = Completer<bool>();
    final chooser = <Object?>[];
    final recovery = recoverFailedStartupOpen(
      libraryOpened: opened.future,
      failure: () => null,
      libraryIsOpen: () => false,
      signInAgain: () async => false,
      showChooser: chooser.add,
    );

    await Future<void>.delayed(Duration.zero);
    expect(chooser, isEmpty);
    opened.complete(false);
    await recovery;

    expect(chooser, [null]);
  });

  for (final kind in [
    OpenLibraryFailureKind.mediaFolderMissing,
    OpenLibraryFailureKind.recordNotFound,
    OpenLibraryFailureKind.remoteUnreachable,
  ]) {
    test(
      'a ${kind.name} library leaves for the chooser with its failure',
      () async {
        final failure = _openFailure(kind);
        final startup = _Startup(opened: false, failure: failure);
        await startup.recover();

        expect(startup.chooser, [same(failure)]);
        expect(startup.signIns, 0);
      },
    );
  }

  test('a library that is open by the time the open fails stays', () async {
    final startup = _Startup(
      opened: false,
      failure: _openFailure(OpenLibraryFailureKind.mediaFolderMissing),
      libraryOpen: true,
    );
    await startup.recover();

    expect(startup.chooser, isEmpty);
  });

  group('a server that rejected the saved password', () {
    final rejected = _openFailure(OpenLibraryFailureKind.remoteNotAuthorized);

    test('opens with the new sign-in', () async {
      final startup = _Startup(
        opened: false,
        failure: rejected,
        signIn: () async => true,
      );
      await startup.recover();

      expect(startup.signIns, 1);
      expect(startup.chooser, isEmpty);
    });

    test('leaves for the chooser when the sign-in is cancelled', () async {
      final startup = _Startup(opened: false, failure: rejected);
      await startup.recover();

      expect(startup.signIns, 1);
      expect(startup.chooser, [same(rejected)]);
    });

    test('leaves for the chooser with the failure of the new open', () async {
      final unreachable = _openFailure(
        OpenLibraryFailureKind.remoteUnreachable,
      );
      final startup = _Startup(
        opened: false,
        failure: rejected,
        signIn: () async => throw unreachable,
      );
      await startup.recover();

      expect(startup.chooser, [same(unreachable)]);
    });
  });
}
