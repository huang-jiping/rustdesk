import 'package:flutter_test/flutter_test.dart';

import 'package:flutter_hbb/models/gamepad_model.dart';

void main() {
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
