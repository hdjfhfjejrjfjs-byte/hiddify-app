import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:hiddify/core/preferences/preferences_provider.dart';
import 'package:hiddify/features/profile/data/profile_parser.dart';
import 'package:hiddify/features/route_rules/notifier/rules_notifier.dart';
import 'package:hiddify/features/settings/data/config_option_data_providers.dart';
import 'package:hiddify/features/settings/data/config_option_repository.dart';
import 'package:hiddify/hiddifycore/generated/v2/config/route_rule.pb.dart';
import 'package:hiddify/singbox/model/singbox_config_enum.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

class _EmptyRules extends RulesNotifier {
  @override
  List<Rule> build() => [];
}

Future<ProviderContainer> _makeContainer([Map<String, Object> stored = const {}]) async {
  SharedPreferences.setMockInitialValues(stored);
  final container = ProviderContainer(overrides: [rulesNotifierProvider.overrideWith(_EmptyRules.new)]);
  await container.read(sharedPreferencesProvider.future);
  return container;
}

void main() {
  group('ConfigOptions.extraSecurityMode defaults to warp_auto', () {
    test('default Extra Security mode and tunnel serialize as warp_auto', () async {
      final container = await _makeContainer();
      addTearDown(container.dispose);

      expect(container.read(ConfigOptions.extraSecurityMode), ChainMode.warp);
      expect(container.read(ConfigOptions.extraSecurityMode).key, 'warp_auto');
      expect(container.read(ConfigOptions.extraSecurityWarpTunnel), WarpTunnel.auto);

      await container.read(ConfigOptions.chainStatus.notifier).update(ChainStatus.extraSecurity);
      final options = container.read(ConfigOptions.singboxConfigOptions);
      expect(options.chainStatus, ChainStatus.extraSecurity);
      expect(options.extraSecurity.mode, 'warp_auto');

      final json = options.toJson();
      expect(json['chain-status'], 'extra_security');
      expect((json['extra-security'] as Map)['mode'], 'warp_auto');
    });

    test('stored legacy or unselectable extra-security-mode falls back to warp_auto', () async {
      for (final storedMode in ['profile', 'warp_auto', 'unknown']) {
        final container = await _makeContainer({'extra-security-mode': storedMode});
        addTearDown(container.dispose);

        expect(container.read(ConfigOptions.extraSecurityMode), ChainMode.warp, reason: storedMode);
        expect(container.read(ConfigOptions.singboxConfigOptions).extraSecurity.mode, 'warp_auto', reason: storedMode);
      }
    });

    test('fullOptionsOverrided keeps warp_auto for both settings and profile overrides', () async {
      final container = await _makeContainer({'chain-status': 'extraSecurity'});
      addTearDown(container.dispose);

      final repo = container.read(configOptionRepositoryProvider);
      final fromSettings = repo
          .fullOptionsOverrided(ProfileParser.profileOverride(populatedHeaders: null, userOverride: null))
          .getOrElse((f) => fail('unexpected failure: $f'));
      expect(fromSettings.chainStatus, ChainStatus.extraSecurity);
      expect(fromSettings.extraSecurity.mode, 'warp_auto');

      final containerFromHeader = await _makeContainer({'extra-security-mode': 'psiphon'});
      addTearDown(containerFromHeader.dispose);
      final fromHeader = containerFromHeader
          .read(configOptionRepositoryProvider)
          .fullOptionsOverrided(
            ProfileParser.profileOverride(populatedHeaders: {'extra-security': 'warp'}, userOverride: null),
          )
          .getOrElse((f) => fail('unexpected failure: $f'));
      expect(fromHeader.chainStatus, ChainStatus.extraSecurity);
      expect(fromHeader.extraSecurity.mode, 'warp_auto');
    });
  });

  group('ConfigOptionRepository.keepOwnDnsBehindExtraSecurity', () {
    Map<String, dynamic> own({String chain = 'off'}) => {
      'remote-dns-address': 'udp://9.9.9.9',
      'direct-dns-address': 'udp://1.1.1.1',
      'chain-status': chain,
    };

    Map<String, dynamic> merge(Map<String, dynamic> own, Map<String, dynamic> override) =>
        ConfigOptionRepository.keepOwnDnsBehindExtraSecurity(
          own,
          ProfileParser.applyProfileOverride({...own}, jsonEncode(override)),
        );

    test('a profile sets the remote DNS while the chain is off', () {
      final merged = merge(own(), {'remote-dns-address': 'https://dns.provider/dns-query'});
      expect(merged['remote-dns-address'], 'https://dns.provider/dns-query');
    });

    test("with extra security on in the settings, the user's own remote DNS stays", () {
      final merged = merge(own(chain: 'extra_security'), {
        'remote-dns-address': 'https://dns.provider/dns-query',
        'direct-dns-address': 'udp://8.8.8.8',
      });
      expect(merged['remote-dns-address'], 'udp://9.9.9.9');
      expect(merged['direct-dns-address'], 'udp://8.8.8.8', reason: 'only the remote DNS is held back');
    });

    test('a profile that turns extra security on does not set the remote DNS either', () {
      final merged = merge(own(), {
        'remote-dns-address': 'https://dns.provider/dns-query',
        'chain-status': 'extra_security',
      });
      expect(merged['chain-status'], 'extra_security');
      expect(merged['remote-dns-address'], 'udp://9.9.9.9');
    });

    test('a profile without a DNS of its own changes nothing', () {
      final merged = merge(own(chain: 'extra_security'), {'connection-test-url': 'https://example.com'});
      expect(merged['remote-dns-address'], 'udp://9.9.9.9');
      expect(merged['connection-test-url'], 'https://example.com');
    });
  });
}
