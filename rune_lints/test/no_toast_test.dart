import 'package:analyzer_testing/analysis_rule/analysis_rule.dart';
import 'package:rune_lints/src/no_toast.dart';
import 'package:test_reflective_loader/test_reflective_loader.dart';

void main() {
  defineReflectiveSuite(() {
    defineReflectiveTests(NoToastTest);
  });
}

@reflectiveTest
class NoToastTest extends AnalysisRuleTest {
  @override
  void setUp() {
    rule = NoToast();
    newPackage('fluent_ui')
      ..addFile('lib/fluent_ui.dart', '''
export 'src/controls/surfaces/info_bar.dart';

class ContentDialog {}
''')
      ..addFile('lib/src/controls/surfaces/info_bar.dart', '''
enum InfoBarSeverity { info }

class InfoBar {
  const InfoBar({this.severity = InfoBarSeverity.info});
  final InfoBarSeverity severity;
}

void displayInfoBar(Object context, {required InfoBar Function() builder}) {}
''');
    super.setUp();
  }

  void test_infoBar_constructed() async {
    await assertDiagnostics(
      '''
import 'package:fluent_ui/fluent_ui.dart';

Object f() => InfoBar();
''',
      [lint(58, 7)],
    );
  }

  void test_infoBar_asTypeLiteral() async {
    await assertDiagnostics(
      '''
import 'package:fluent_ui/fluent_ui.dart';

Type f() => InfoBar;
''',
      [lint(56, 7)],
    );
  }

  void test_displayInfoBar_called() async {
    await assertDiagnostics(
      '''
import 'package:fluent_ui/fluent_ui.dart' as fluent;

void f(Object context) {
  fluent.displayInfoBar(
    context,
    builder: () => const fluent.InfoBar(
      severity: fluent.InfoBarSeverity.info,
    ),
  );
}
''',
      [lint(88, 14), lint(149, 7), lint(181, 15)],
    );
  }

  void test_dialog_allowed() async {
    await assertNoDiagnostics('''
import 'package:fluent_ui/fluent_ui.dart';

Object f() => ContentDialog();
''');
  }

  void test_ownDeclaration_allowed() async {
    await assertNoDiagnostics('''
class InfoBar {}

Object f() => InfoBar();
''');
  }
}
