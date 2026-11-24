import 'package:flutter/widgets.dart';

import '../../bindings/bindings.dart';
import '../../providers/request_capabilities.dart';

/// Whether the open library keeps mixes that a track or collection can be
/// added to. A target that does not, such as a Subsonic server, gets no mix
/// entries in its menus.
bool offersMixes(BuildContext context) {
  bool available(
    RequestCapability Function(RequestCapabilitiesProvider) select,
  ) =>
      capabilityForRequest(context, select, listen: false)
          is RequestCapabilityAvailable;
  return available((capabilities) => capabilities.fetchAllMixesRequest) &&
      available((capabilities) => capabilities.addItemToMixRequest);
}
