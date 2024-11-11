const String resetColor = '\x1B[0m';
const String grayColor = '\x1B[90m';
const String redColor = '\x1B[91m';
const String yellowColor = '\x1B[93m';
const String blueColor = '\x1B[94m';
const String greenColor = '\x1B[92m';
const String magentaColor = '\x1B[95m';

void log(String type, String color, String content) {
  final timestamp = DateTime.now().toUtc().toIso8601String();
  final formattedLog =
      '${'\b \b' * 9}$grayColor$timestamp$resetColor  $color$type$resetColor$magentaColor flutter: $resetColor$content';

  // ignore: avoid_print
  print(formattedLog);
}

void debug$(String content) {
  log('DEBUG', blueColor, content);
}

void info$(String content) {
  log('INFO', greenColor, content);
}

void warn$(String content) {
  log('WARN', yellowColor, content);
}

void error$(String content) {
  log('ERROR', redColor, content);
}
