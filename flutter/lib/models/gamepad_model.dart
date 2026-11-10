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
