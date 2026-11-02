import 'package:flutter_test/flutter_test.dart';

import '../../bindings/bindings.dart';
import '../../utils/api/fetch_mix_queries_by_mix_id.dart';

StoredMixQuery clause(String operator, String parameter, bool available) =>
    StoredMixQuery(
      clauseId: '00000000-0000-4000-8000-000000000001',
      operator: operator,
      parameter: parameter,
      targetUuid: null,
      available: available,
    );

void main() {
  test('an unavailable selection cannot silently broaden a mix', () {
    expect(
      () => executableMixQueries([
        clause('lib::all', 'true', true),
        clause('lib::album', 'missing-album', false),
      ]),
      throwsA(isA<UnavailableMixSelection>()),
    );
  });

  test('an unbound directory cannot become a root-directory query', () {
    expect(
      () => executableMixQueries([clause('lib::directory.deep', '', false)]),
      throwsA(isA<UnavailableMixSelection>()),
    );
  });

  test('executable selections preserve order and repeated clauses', () {
    expect(
      executableMixQueries([
        clause('lib::track', '7', true),
        clause('lib::track', '7', true),
        clause('pipe::limit', '10', true),
      ]),
      [('lib::track', '7'), ('lib::track', '7'), ('pipe::limit', '10')],
    );
  });
}
