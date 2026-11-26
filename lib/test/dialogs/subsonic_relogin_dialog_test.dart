import 'package:flutter_test/flutter_test.dart';
import '../../bindings/bindings.dart';
import '../../utils/dialogs/subsonic/subsonic_relogin_dialog.dart';

void main() {
  test('only a record that keeps a credential asks for it again', () {
    const records = [
      LibraryRecordSummary(
        recordId: 'kept-credential',
        alias: 'Server',
        kind: LibraryRecordKindSummarySubsonic(
          endpoint: 'https://music.example',
          account: 'listener',
        ),
      ),
      LibraryRecordSummary(
        recordId: 'signed-in-here',
        alias: 'Service',
        kind: LibraryRecordKindSummaryAppleMusic(),
        signedIn: false,
      ),
    ];

    expect(signsInWithKeptCredential(records, 'kept-credential'), isTrue);
    // Its sign-in lives on this device, and the platform signs it in again.
    expect(signsInWithKeptCredential(records, 'signed-in-here'), isFalse);
    // A record of another device signs in there.
    expect(signsInWithKeptCredential(records, 'elsewhere'), isFalse);
  });
}
