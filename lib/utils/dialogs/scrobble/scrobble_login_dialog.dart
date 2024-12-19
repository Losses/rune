import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../../messages/all.dart';
import '../../../providers/scrobble.dart';

import '../../l10n.dart';

import '../information/error.dart';

import 'utils/scrobble_login_controller.dart';
import 'widgets/scrobble_login_form.dart';

class ScrobbleLoginDialog extends StatefulWidget {
  final String serviceName;
  final String title;
  final void Function(LoginRequestItem?) $close;

  const ScrobbleLoginDialog({
    super.key,
    required this.serviceName,
    required this.title,
    required this.$close,
  });

  @override
  ScrobbleLoginDialogState createState() => ScrobbleLoginDialogState();
}

class ScrobbleLoginDialogState extends State<ScrobbleLoginDialog> {
  bool isLoading = false;
  late ScrobbleLoginController controller;

  @override
  void initState() {
    super.initState();
    controller = ScrobbleLoginController();
  }

  @override
  void dispose() {
    controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);
    final scrobble = Provider.of<ScrobbleProvider>(context);

    return ContentDialog(
      title: Text(widget.title),
      content: ScrobbleLoginForm(
        serviceName: widget.serviceName,
        controller: controller,
      ),
      actions: [
        Button(
          onPressed: isLoading ? null : () => widget.$close(null),
          child: Text(s.cancel),
        ),
        FilledButton(
          onPressed: isLoading
              ? null
              : () async {
                  setState(() {
                    isLoading = true;
                  });

                  final loginRequestItem =
                      controller.toLoginRequestItem(widget.serviceName);

                  try {
                    scrobble.login(loginRequestItem);
                    widget.$close(loginRequestItem);
                  } catch (e) {
                    showErrorDialog(
                      context: context,
                      title: s.loginFailed,
                      subtitle: s.loginFailedSubtitle,
                      errorMessage: e.toString(),
                    );
                  }

                  setState(() {
                    isLoading = false;
                  });
                },
          child: Text(s.login),
        ),
      ],
    );
  }
}
