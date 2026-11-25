/// The argument that selects a settings profile.
const profileArgument = '--profile';

/// The settings profile selected at launch, or null when the argument is absent
/// or has no value. A missing value leaves the default profile selected.
String? profileAtLaunch(List<String> arguments) {
  final profileIndex = arguments.indexOf(profileArgument);
  if (profileIndex == -1 || profileIndex + 1 == arguments.length) return null;
  return arguments[profileIndex + 1];
}
