import 'dart:math';

import 'package:fluentui_system_icons/fluentui_system_icons.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:gap/gap.dart';
import 'package:go_router/go_router.dart';
import 'package:hiddify/core/localization/translations.dart';
import 'package:hiddify/core/preferences/general_preferences.dart';
import 'package:hiddify/core/router/dialog/dialog_notifier.dart';
import 'package:hiddify/core/widget/adaptive_icon.dart';
import 'package:hiddify/features/connection/model/connection_status.dart';
import 'package:hiddify/features/connection/notifier/connection_notifier.dart';
import 'package:hiddify/features/profile/notifier/active_profile_notifier.dart';
import 'package:hiddify/features/proxy/keep_top/keep_top.dart';
import 'package:hiddify/features/proxy/keep_top/keep_top_widgets.dart';
import 'package:hiddify/features/proxy/model/proxy_entity.dart';
import 'package:hiddify/features/proxy/overview/offline_proxies_provider.dart';
import 'package:hiddify/features/proxy/overview/proxies_header.dart';
import 'package:hiddify/features/proxy/overview/proxies_overview_notifier.dart';
import 'package:hiddify/features/proxy/widget/proxy_tile.dart';
import 'package:hiddify/hiddifycore/generated/v2/hcore/hcore.pb.dart';
import 'package:hiddify/utils/utils.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

class ProxiesOverviewPage extends HookConsumerWidget with PresLogger {
  const ProxiesOverviewPage({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = ref.watch(translationsProvider).requireValue;

    final proxies = ref.watch(proxiesOverviewNotifierProvider);
    final sortBy = ref.watch(proxiesSortNotifierProvider);
    // the proxies keep top left at rest, under the list: shown at first, then folded or not as the user left them
    final showResting = ref.watch(Preferences.showSleepingProxies);
    final activeProfile = ref.watch(activeProfileProvider).valueOrNull;
    final connectionStatus = ref.watch(connectionNotifierProvider).valueOrNull;
    final serviceRunning = ref.watch(serviceRunningProvider);
    final offlineProxies = ref.watch(activeProfileProxiesProvider);
    final pendingSelection = ref.watch(pendingProxySelectionProvider);

    final pageFocusNode = useFocusNode();

    final rawOfflineProxies = offlineProxies.valueOrNull;
    final sortedOfflineProxies = useMemoized(() {
      if (rawOfflineProxies == null || rawOfflineProxies.isEmpty) return const <OutboundInfo>[];
      final outbounds = rawOfflineProxies.map(offlineProxyToOutboundInfo).toList(growable: false);
      return sortOfflineProxies(outbounds, sortBy);
    }, [rawOfflineProxies, sortBy]);

    final liveGroup = proxies.valueOrNull;
    final isConnected = liveGroup != null;
    final isConnecting =
        !isConnected &&
        (connectionStatus is Connecting || (connectionStatus?.isRunning ?? false) || pendingSelection != null);

    final profileProxyTotal = activeProfile == null
        ? null
        : ref.watch(profileProxyCountProvider(activeProfile.id)).valueOrNull;
    final liveTotalCount = liveGroup == null
        ? (rawOfflineProxies?.length ?? profileProxyTotal ?? 0)
        : () {
            final nonGroup = liveGroup.items.where((e) => !e.isGroup).length;
            return nonGroup > 0 ? nonGroup : liveGroup.items.length;
          }();
    final totalCount = profileProxyTotal != null && profileProxyTotal > 0 ? profileProxyTotal : liveTotalCount;

    void handleEscape() {
      if (context.canPop()) {
        context.pop();
      } else {
        context.goNamed('home');
      }
    }

    Widget buildOfflineBody() {
      if (activeProfile == null) {
        return Center(
          child: Padding(
            padding: const EdgeInsets.all(24),
            child: Text(t.pages.proxies.empty, textAlign: TextAlign.center),
          ),
        );
      }

      return offlineProxies.when(
        loading: () => const Center(child: CircularProgressIndicator()),
        error: (_, _) => Center(
          child: Padding(
            padding: const EdgeInsets.all(24),
            child: Text(
              serviceRunning ? t.pages.proxies.empty : t.pages.proxies.connectToSeeProxies,
              textAlign: TextAlign.center,
            ),
          ),
        ),
        data: (items) {
          if (items.isEmpty) {
            return Center(
              child: Padding(
                padding: const EdgeInsets.all(24),
                child: Text(
                  serviceRunning ? t.pages.proxies.empty : t.pages.proxies.connectToSeeProxies,
                  textAlign: TextAlign.center,
                ),
              ),
            );
          }

          return LayoutBuilder(
            builder: (context, constraints) {
              final width = constraints.maxWidth;
              final crossAxisCount = PlatformUtils.isMobile && width < 600 ? 1 : max(1, (width / 420).floor());
              return CustomScrollView(
                slivers: [
                  SliverGrid.builder(
                    itemCount: sortedOfflineProxies.length,
                    gridDelegate: SliverGridDelegateWithFixedCrossAxisCount(
                      crossAxisCount: crossAxisCount,
                      mainAxisExtent: 72,
                    ),
                    itemBuilder: (context, index) {
                      final proxy = sortedOfflineProxies[index];
                      final isPendingTarget =
                          pendingSelection != null &&
                          pendingSelection.profileId == activeProfile.id &&
                          pendingSelection.outboundTag == proxy.tag;
                      return ProxyTile(
                        proxy,
                        key: ValueKey(proxy.tag),
                        selected: isPendingTarget,
                        showPing: isPendingTarget,
                        testing: isPendingTarget,
                        testable: false,
                        // keep non-tapped offline rows calm (no shimmering flag placeholder),
                        // while allowing the tapped connecting target to animate its loading logo
                        quiet: !isPendingTarget,
                        onTap: () =>
                            ref.read(proxiesOverviewNotifierProvider.notifier).connectAndSelectProxy(proxy.tag),
                      );
                    },
                  ),
                  const SliverPadding(padding: EdgeInsets.only(bottom: 24)),
                ],
              );
            },
          );
        },
      );
    }

    Widget buildLiveBody(OutboundGroup group) {
      final inUse = group.items.where((e) => !e.resting).toList();
      final resting = group.items.where((e) => e.resting).toList();

      return LayoutBuilder(
        builder: (context, constraints) {
          final width = constraints.maxWidth;
          // columns wide enough for a proxy's name: two on a usual window, more only when wider
          final crossAxisCount = PlatformUtils.isMobile && width < 600 ? 1 : max(1, (width / 420).floor());
          final listed = inUse.where((e) => e.tag != lowestGroupTag && e.tag != balanceGroupTag).length;

          Widget grid(List<OutboundInfo> items) => SliverGrid.builder(
            itemCount: items.length,
            gridDelegate: SliverGridDelegateWithFixedCrossAxisCount(crossAxisCount: crossAxisCount, mainAxisExtent: 72),
            itemBuilder: (context, index) {
              final proxy = items[index];
              return ProxyTile(
                proxy,
                // a row keeps its own proxy when the list re-sorts, so a pop plays only on arrival
                key: ValueKey(proxy.tag),
                selected: group.selected == proxy.tag,
                // a pick goes through hundreds of proxies, a row or two change every second:
                // their values change in place, nothing pops or loops
                quiet: group.keepTopPicking,
                // selecting one at rest loads it and tests it
                onTap: () async {
                  await ref.read(proxiesOverviewNotifierProvider.notifier).changeProxy(group.tag, proxy.tag);
                },
              );
            },
          );

          // a big profile: how many of its proxies run, and what changes it
          final keepTop = SliverToBoxAdapter(
            child: KeepTopFooter(listed: listed, picking: group.keepTopPicking, proxies: group.items),
          );

          return CustomScrollView(
            slivers: [
              if (group.keepTopPicking) ...[
                // under the proxies that run; over the ones still being tested, none of which
                // is kept yet
                keepTop,
                const SliverPadding(padding: EdgeInsets.only(top: 12)),
                grid(inUse),
              ] else ...[
                grid(inUse),
                keepTop,
              ],
              if (resting.isNotEmpty) ...[
                SliverToBoxAdapter(
                  child: RestingProxiesHeader(
                    count: resting.length,
                    shown: showResting,
                    onToggle: () => ref.read(Preferences.showSleepingProxies.notifier).update(!showResting),
                  ),
                ),
                if (showResting) grid(resting),
              ],
              // room for the buttons over the list's end
              const SliverPadding(padding: EdgeInsets.only(bottom: 140)),
            ],
          );
        },
      );
    }

    return CallbackShortcuts(
      bindings: {const SingleActivator(LogicalKeyboardKey.escape): handleEscape},
      child: Focus(
        focusNode: pageFocusNode,
        autofocus: true,
        child: Scaffold(
          appBar: AppBar(
            title: Text(t.pages.proxies.title),
            actions: [
              PopupMenuButton<ProxiesSort>(
                initialValue: sortBy,
                onSelected: ref.read(proxiesSortNotifierProvider.notifier).update,
                icon: const Icon(FluentIcons.arrow_sort_24_regular),
                tooltip: t.pages.proxies.sort,
                itemBuilder: (context) {
                  return [...ProxiesSort.values.map((e) => PopupMenuItem(value: e, child: Text(e.present(t))))];
                },
              ),
              // "Signal style" (off by default) shows pings as the signal instead of numbers
              PopupMenuButton<bool>(
                onSelected: (_) => ref.read(proxyPingStyleNotifierProvider.notifier).toggle(),
                icon: Icon(AdaptiveIcon(context).more),
                tooltip: MaterialLocalizations.of(context).showMenuTooltip,
                itemBuilder: (context) => [
                  // a null value would close the menu as cancelled
                  PopupMenuItem(
                    value: true,
                    child: Row(
                      children: [
                        Checkbox(
                          value: ref.read(proxyPingStyleNotifierProvider) == ProxyPingStyle.signal,
                          // a tap on the box itself does what a tap on the item does
                          onChanged: (_) => Navigator.pop(context, true),
                        ),
                        const Gap(8),
                        Text(t.pages.proxies.signalStyle),
                      ],
                    ),
                  ),
                ],
              ),
              const Gap(8),
            ],
          ),
          floatingActionButton: liveGroup == null
              ? null
              : Column(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.end,
                  children: [
                    // the speed of the proxies listed: what it tests and the data it takes show first, and
                    // it starts from there
                    FloatingActionButton.small(
                      heroTag: 'upload-test',
                      onPressed: () async {
                        final count = liveGroup.items.where((e) => !e.isGroup && !e.resting).length;
                        if (await ref.read(dialogNotifierProvider.notifier).showSpeedTest(count: count)) {
                          await ref.read(proxiesOverviewNotifierProvider.notifier).speedTestAll();
                        }
                      },
                      tooltip: t.pages.proxies.speedTest.title,
                      child: const Icon(Icons.speed_rounded, size: 26),
                    ),
                    const Gap(12),
                    FloatingActionButton(
                      onPressed: () async => await ref.read(proxiesOverviewNotifierProvider.notifier).urlTest("select"),
                      tooltip: t.pages.proxies.testDelay,
                      child: const Icon(FluentIcons.flash_24_filled),
                    ),
                  ],
                ),
          body: Column(
            children: [
              ProxiesHeader(
                activeProfile: activeProfile,
                isConnected: isConnected,
                isConnecting: isConnecting,
                totalCount: totalCount,
              ),
              Expanded(
                child: proxies.when(
                  data: (group) => group != null ? buildLiveBody(group) : buildOfflineBody(),
                  error: (error, stackTrace) => Center(
                    child: Padding(
                      padding: const EdgeInsets.all(24),
                      child: Column(
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          Text(t.pages.proxies.loadFailed, textAlign: TextAlign.center),
                          const Gap(12),
                          TextButton(
                            onPressed: () => ref.invalidate(proxiesOverviewNotifierProvider),
                            child: Text(t.connection.tapToRetry),
                          ),
                        ],
                      ),
                    ),
                  ),
                  loading: () => pendingSelection != null || !serviceRunning
                      ? buildOfflineBody()
                      : const Center(child: CircularProgressIndicator()),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
