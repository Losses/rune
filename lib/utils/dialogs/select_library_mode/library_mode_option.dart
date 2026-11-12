import '../../../bindings/bindings.dart';

class LibraryModeOption {
  final String title;
  final String subtitle;
  final LibraryInitializeMode value;

  const LibraryModeOption({
    required this.title,
    required this.subtitle,
    required this.value,
  });
}
