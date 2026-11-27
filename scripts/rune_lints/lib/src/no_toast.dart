import 'package:analyzer/analysis_rule/analysis_rule.dart';
import 'package:analyzer/analysis_rule/rule_context.dart';
import 'package:analyzer/analysis_rule/rule_visitor_registry.dart';
import 'package:analyzer/dart/ast/ast.dart';
import 'package:analyzer/dart/ast/visitor.dart';
import 'package:analyzer/dart/element/element.dart';
import 'package:analyzer/error/error.dart';

/// The libraries that declare toasts: Fluent UI's `InfoBar` and
/// `displayInfoBar`, and Material's `SnackBar`.
const _toastLibraries = {
  'package:fluent_ui/src/controls/surfaces/info_bar.dart',
  'package:flutter/src/material/snack_bar.dart',
  'package:flutter/src/material/snack_bar_theme.dart',
};

/// Rune tells the user things in a dialog, never in a toast. This reports
/// every use of a name the toast libraries declare, in tests as well.
class NoToast extends AnalysisRule {
  static const LintCode code = LintCode(
    'no_toast',
    "'{0}' is a toast, which Rune does not use.",
    correctionMessage:
        'Show a dialog instead, such as showInformationDialog or '
        'showErrorDialog.',
    severity: DiagnosticSeverity.ERROR,
  );

  NoToast()
    : super(
        name: 'no_toast',
        description:
            'Toasts (InfoBar, displayInfoBar, SnackBar) are not part of '
            "Rune's design. Use a dialog.",
      );

  @override
  DiagnosticCode get diagnosticCode => code;

  @override
  void registerNodeProcessors(
    RuleVisitorRegistry registry,
    RuleContext context,
  ) {
    final visitor = _Visitor(this);
    registry.addSimpleIdentifier(this, visitor);
    registry.addNamedType(this, visitor);
  }
}

class _Visitor extends SimpleAstVisitor<void> {
  final AnalysisRule rule;

  _Visitor(this.rule);

  /// Whether [element] is declared at the top of a toast library. Their
  /// members are only reached through one of those names.
  bool _isToast(Element? element) {
    final library = element?.enclosingElement;
    return library is LibraryElement &&
        _toastLibraries.contains(library.uri.toString());
  }

  @override
  void visitSimpleIdentifier(SimpleIdentifier node) {
    if (node.inDeclarationContext()) return;
    if (_isToast(node.element)) {
      rule.reportAtNode(node, arguments: [node.name]);
    }
  }

  @override
  void visitNamedType(NamedType node) {
    if (_isToast(node.element)) {
      rule.reportAtToken(node.name, arguments: [node.name.lexeme]);
    }
  }
}
