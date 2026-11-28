import 'package:flutter/material.dart';
import 'package:gap/gap.dart';
import 'package:hiddify/core/localization/translations.dart';
import 'package:hiddify/features/stats/notifier/stats_notifier.dart';
import 'package:hiddify/features/stats/widget/stats_card.dart';
import 'package:hiddify/utils/number_formatters.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

class SideBarStatsOverview extends HookConsumerWidget {
  const SideBarStatsOverview({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = ref.watch(translationsProvider).requireValue;

    // the four numbers shown, and nothing else of the core's status: it comes every second with
    // its memory and goroutine counts, which change every time, and each rebuild here is a frame
    final (uplink, downlink, uplinkTotal, downlinkTotal) = ref.watch(
      statsNotifierProvider.select((stats) {
        final s = stats.asData?.value;
        return s == null
            ? (0, 0, 0, 0)
            : (s.uplink.toInt(), s.downlink.toInt(), s.uplinkTotal.toInt(), s.downlinkTotal.toInt());
      }),
    );

    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 16, horizontal: 16),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          StatsCard(
            title: t.components.stats.trafficLive,
            stats: [
              (
                label: const Text("↑", style: TextStyle(color: Colors.green)),
                data: Text(uplink.speed()),
                semanticLabel: t.components.stats.uplink,
              ),
              (
                label: Text("↓", style: TextStyle(color: Theme.of(context).colorScheme.error)),
                data: Text(downlink.speed()),
                semanticLabel: t.components.stats.downlink,
              ),
            ],
          ),
          const Gap(8),
          StatsCard(
            title: t.components.stats.trafficTotal,
            stats: [
              (
                label: const Text("↑", style: TextStyle(color: Colors.green)),
                data: Text(uplinkTotal.size()),
                semanticLabel: t.components.stats.uplink,
              ),
              (
                label: Text("↓", style: TextStyle(color: Theme.of(context).colorScheme.error)),
                data: Text(downlinkTotal.size()),
                semanticLabel: t.components.stats.downlink,
              ),
            ],
          ),
        ],
      ),
    );
  }
}
