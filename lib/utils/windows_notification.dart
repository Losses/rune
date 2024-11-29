import 'dart:io';

import 'rune_log.dart';

void showNotification(String title, String text,
    [String iconType = 'Info']) async {
  title = title.replaceAll("'", "''");
  text = text.replaceAll("'", "''");
  iconType = iconType.replaceAll("'", "''");

  final script = '''
Add-Type -AssemblyName System.Windows.Forms
\$NotifyIcon = [System.Windows.Forms.NotifyIcon]::new()
\$NotifyIcon.Icon = [System.Drawing.Icon]::ExtractAssociatedIcon((Get-Process -Id \$PID).Path)
\$NotifyIcon.Visible = \$true
\$NotifyIcon.ShowBalloonTip(5000, '$title', '$text', '$iconType')
''';

  final process = await Process.start(
    'powershell',
    ['-Command', script],
    runInShell: true,
  );

  await stdout.addStream(process.stdout);
  await stderr.addStream(process.stderr);

  final exitCode = await process.exitCode;
  if (exitCode != 0) {
    error$('PowerShell script failed with exit code $exitCode');
  }
}
