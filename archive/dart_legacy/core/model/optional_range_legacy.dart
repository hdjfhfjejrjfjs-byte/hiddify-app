// ARCHIVED LEGACY DART IMPLEMENTATION (Phase 2)
// Original path: lib/core/model/optional_range.dart
// Replaced by:   rust/crates/hiddify-config (OptionalRange::parse, format)

({int? min, int? max}) legacyParseOptionalRange(String input, {bool allowEmpty = false}) => switch (input.split("-")) {
  [final String val] when val.isEmpty && allowEmpty => (min: null, max: null),
  [final String min] => (min: int.parse(min), max: null),
  [final String min, final String max] => (min: int.parse(min), max: int.parse(max)),
  _ => throw Exception("Invalid range: $input"),
};

String legacyFormatOptionalRange(int? min, int? max) => [if (min != null) min, if (max != null) max].join("-");
