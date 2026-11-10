import 'dart:io';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:permission_handler/permission_handler.dart';
import 'package:provider/provider.dart';

import '../bindings/bindings.dart';
import '../providers/library_manager.dart';
import '../providers/library_registry.dart';
import '../providers/local_folder_picker.dart';
import 'api/add_trusted_server.dart';
import 'api/register_device_on_server.dart';
import 'api/request_failure.dart';
import 'dialogs/failed_to_initialize_library.dart';
import 'dialogs/information/error.dart';
import 'dialogs/information/information.dart';
import '../screens/settings_neighbors/utils/show_search_remote_device_dialog.dart';
import '../screens/settings_neighbors/utils/show_server_status_dialog.dart';
import 'l10n.dart';
import 'library_registry_messages.dart';
import 'router/navigation.dart';
import 'rune_log.dart';

Future<void> addLocalFolderLibrary(
  BuildContext context, {
  bool navigateIfFailed = false,
  Future<String?> Function()? pickDirectory,
}) async {
  final registry = Provider.of<LibraryRegistryProvider>(context, listen: false);
  final libraryManager = Provider.of<LibraryManagerProvider>(
    context,
    listen: false,
  );

  if (!await requestAndroidPermission()) return;
  if (!context.mounted) return;
  final path =
      await (pickDirectory ??
          Provider.of<LocalFolderPickerProvider>(
            context,
            listen: false,
          ).pickDirectory)();
  info$("Path selected $path");
  if (path == null || !context.mounted) return;

  final alias = path.split('/').where((part) => part.isNotEmpty).last;
  try {
    final record = await registerAndOpenLocalFolder(
      context,
      registry,
      path,
      alias,
    );
    if (record == null) return;
    libraryManager.scanLibrary(record.recordId, isInitializeTask: true);
  } catch (error) {
    if (!context.mounted) return;
    await showFailedToInitializeLibrary(
      context,
      error is LibraryRegistryFailure
          ? libraryRegistryFailureMessage(S.of(context), error.kind)
          : S.of(context).unknownError,
    );
    if (navigateIfFailed) $$replace('/');
  }
}

Future<void> addRuneDeviceLibrary(
  BuildContext context, {
  Future<(DiscoveredDeviceMessage, bool?)?> Function(BuildContext)? showSearch,
}) async {
  final libraryRegistry = Provider.of<LibraryRegistryProvider>(
    context,
    listen: false,
  );
  final s = S.of(context);
  final response = await (showSearch ?? showSearchRemoteDeviceDialog)(context);
  if (response == null) return;
  final (device, correct) = response;
  if (correct == null) return;

  if (!correct) {
    if (!context.mounted) return;
    showInformationDialog(
      context: context,
      title: s.pairingFailureTitle,
      subtitle: s.pairingFailureMessage,
    );
    return;
  }

  try {
    await addTrustedServer(device.fingerprint, device.ips);
  } catch (error) {
    if (!context.mounted) return;
    showErrorDialog(
      context: context,
      title: s.unknownError,
      errorMessage: requestErrorText(S.of(context), error),
    );
    return;
  }

  try {
    await registerDeviceOnServer(device.ips, device.fingerprint);
  } catch (error) {
    if (!context.mounted) return;
    showErrorDialog(
      context: context,
      title: s.unknownError,
      errorMessage: requestErrorText(S.of(context), error),
    );
    return;
  }

  if (!context.mounted) return;
  final serverStatus = await showServerStatusDialog(
    context,
    device.ips,
    device.fingerprint,
  );
  if (serverStatus == null) return;
  if (!context.mounted) return;
  if (serverStatus.status == ClientStatus.blocked) {
    await showErrorDialog(
      context: context,
      title: s.clientBlockedTitle,
      errorMessage: s.clientBlockedMessage,
    );
    return;
  }

  if (!context.mounted) return;
  try {
    final registered = await libraryRegistry.registerRemoteControl(
      device.fingerprint,
      device.alias,
    );
    if (registered.record != null) {
      await libraryRegistry.open(registered.record!.recordId);
    }
  } catch (error) {
    if (!context.mounted) return;
    await showErrorDialog(
      context: context,
      title: s.unknownError,
      errorMessage: error is LibraryRegistryFailure
          ? libraryRegistryFailureMessage(s, error.kind)
          : requestErrorText(s, error),
    );
  }
}

Future<bool> requestAndroidPermission() async {
  if (Platform.isAndroid) {
    final status = await Permission.manageExternalStorage.status;
    if (status.isDenied) {
      return await Permission.manageExternalStorage.request().isGranted;
    }
  }
  return true;
}
