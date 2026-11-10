class Features {
  bool privacyMode = false;
  bool gamepad = false;

  void update(Map<String, dynamic> features) {
    privacyMode = features['privacy_mode'] == true;
    gamepad = features['gamepad'] == true;
  }
}

class GamepadModel {
  final String sessionId;
  String status = 'STOPPED';
  int slot = 0;
  bool localConnected = false;

  GamepadModel(this.sessionId);

  static bool showMenu({
    required bool nativeWindows,
    required bool peerWindows,
    required bool remoteSession,
    required bool capability,
  }) =>
      nativeWindows && peerWindows && remoteSession && capability;

  static bool optionEnabled(String? value) => value == 'Y';

  bool canToggle({required bool keyboardEnabled}) =>
      keyboardEnabled && status != 'DISABLED_BY_REMOTE';

  bool shouldRestore(String? option,
          {required bool enabled, required bool keyboardEnabled}) =>
      optionEnabled(option) &&
      !enabled &&
      keyboardEnabled &&
      status == 'STOPPED';

  bool isChecked(bool enabled) =>
      enabled && const {'STOPPED', 'READY', 'ACTIVE'}.contains(status);

  String get statusText => switch (status) {
        'STOPPED' => 'Disconnected',
        'READY' => 'Ready',
        'ACTIVE' => 'Gamepad is active',
        'DISABLED_BY_REMOTE' => 'Gamepad is disabled on the remote device',
        'DRIVER_MISSING' => 'Remote gamepad driver is unavailable',
        'VERSION_MISMATCH' => 'Remote gamepad version is incompatible',
        'NO_FREE_SLOT' => 'No free remote gamepad slot',
        'BUSY' => 'Remote gamepad is in use',
        'ACCESS_DENIED' => 'Remote gamepad access denied',
        _ => 'Remote gamepad error',
      };

  String get localStatusText => localConnected ? 'Connected' : 'Disconnected';

  bool update(Map<String, dynamic> event) {
    if (event['session_id'] != sessionId) return false;
    final nextStatus = event['status'];
    const statuses = {
      'STOPPED',
      'READY',
      'ACTIVE',
      'DISABLED_BY_REMOTE',
      'DRIVER_MISSING',
      'VERSION_MISMATCH',
      'NO_FREE_SLOT',
      'BUSY',
      'ACCESS_DENIED',
      'INTERNAL_ERROR',
    };
    status = statuses.contains(nextStatus) ? nextStatus : 'INTERNAL_ERROR';
    slot = int.tryParse('${event['slot']}') ?? 0;
    localConnected =
        event['local_connected'] == 'true' || event['local_connected'] == true;
    return true;
  }

  void reset() {
    status = 'STOPPED';
    slot = 0;
    localConnected = false;
  }
}
