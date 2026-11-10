import 'package:flutter_test/flutter_test.dart';

import 'package:flutter_hbb/models/gamepad_model.dart';

void main() {
  test('gamepad menu is restricted to supported native Windows remote sessions',
      () {
    for (final flags in [
      [true, true, true, true, true],
      [false, true, true, true, false],
      [true, false, true, true, false],
      [true, true, false, true, false],
      [true, true, true, false, false],
    ]) {
      expect(
          GamepadModel.showMenu(
              nativeWindows: flags[0],
              peerWindows: flags[1],
              remoteSession: flags[2],
              capability: flags[3]),
          flags[4]);
    }
  });

  test('gamepad menu reflects remote status and clears failed runtime intent',
      () {
    final model = GamepadModel('session');
    for (final entry in <String, (bool, bool, String)>{
      'STOPPED': (true, true, 'Disconnected'),
      'READY': (true, true, 'Ready'),
      'ACTIVE': (true, true, 'Gamepad is active'),
      'DISABLED_BY_REMOTE': (
        false,
        false,
        'Gamepad is disabled on the remote device'
      ),
      'DRIVER_MISSING': (true, false, 'Remote gamepad driver is unavailable'),
      'VERSION_MISMATCH': (
        true,
        false,
        'Remote gamepad version is incompatible'
      ),
      'NO_FREE_SLOT': (true, false, 'No free remote gamepad slot'),
      'BUSY': (true, false, 'Remote gamepad is in use'),
      'ACCESS_DENIED': (true, false, 'Remote gamepad access denied'),
      'INTERNAL_ERROR': (true, false, 'Remote gamepad error'),
    }.entries) {
      model.update({
        'session_id': 'session',
        'status': entry.key,
        'local_connected': entry.key == 'ACTIVE',
      });
      expect(model.canToggle(keyboardEnabled: true), entry.value.$1,
          reason: entry.key);
      expect(model.canToggle(keyboardEnabled: false), isFalse);
      expect(model.isChecked(true), entry.value.$2, reason: entry.key);
      expect(model.isChecked(false), isFalse);
      expect(model.statusText, entry.value.$3, reason: entry.key);
      expect(model.localStatusText,
          entry.key == 'ACTIVE' ? 'Connected' : 'Disconnected');
    }
  });

  test(
      'saved gamepad options require explicit Y and do not share session state',
      () {
    for (final value in <String?>[null, '', 'N', 'true', 'y']) {
      expect(GamepadModel.optionEnabled(value), isFalse);
    }
    expect(GamepadModel.optionEnabled('Y'), isTrue);
    final first = GamepadModel('first');
    final second = GamepadModel('second');
    first.update({'session_id': 'first', 'status': 'DRIVER_MISSING'});
    expect(first.shouldRestore('Y', enabled: false, keyboardEnabled: true),
        isFalse);
    expect(second.shouldRestore('Y', enabled: false, keyboardEnabled: true),
        isTrue);
    expect(second.shouldRestore('', enabled: false, keyboardEnabled: true),
        isFalse);
    expect(second.shouldRestore('Y', enabled: true, keyboardEnabled: true),
        isFalse);
    expect(second.shouldRestore('Y', enabled: false, keyboardEnabled: false),
        isFalse);
    expect(first.isChecked(true), isFalse);
    expect(second.isChecked(true), isTrue);
    expect(second.statusText, 'Disconnected');
  });

  test('older peers default to no gamepad capability', () {
    final features = Features();
    expect(features.gamepad, isFalse);
    features.update({'privacy_mode': true, 'gamepad': true});
    expect(features.gamepad, isTrue);
    expect(features.privacyMode, isTrue);
    features.update({'privacy_mode': false});
    expect(features.gamepad, isFalse);
    expect(features.privacyMode, isFalse);
  });

  test('gamepad status only updates its session', () {
    final first = GamepadModel('first-session');
    final second = GamepadModel('second-session');
    final event = <String, dynamic>{
      'session_id': 'first-session',
      'status': 'ACTIVE',
      'slot': '0',
      'local_connected': 'true',
    };
    expect(first.update(event), isTrue);
    expect(second.update(event), isFalse);
    expect(first.status, 'ACTIVE');
    expect(first.slot, 0);
    expect(first.localConnected, isTrue);
    expect(second.status, 'STOPPED');
    expect(second.localConnected, isFalse);
  });

  test('unknown gamepad status fails closed and clear resets the session', () {
    final model = GamepadModel('session');
    model.update({
      'session_id': 'session',
      'status': 'FUTURE_STATUS',
      'slot': '0',
      'local_connected': 'false',
    });
    expect(model.status, 'INTERNAL_ERROR');
    model.reset();
    expect(model.status, 'STOPPED');
    expect(model.localConnected, isFalse);
  });
}
