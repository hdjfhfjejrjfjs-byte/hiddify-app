import 'package:dart_mappable/dart_mappable.dart';
import 'package:freezed_annotation/freezed_annotation.dart';
import 'package:hiddify/core/localization/translations.dart';
import 'package:hiddify/core/rust/hiddify_rust.dart';

part 'optional_range.mapper.dart';

@MappableClass()
class OptionalRange with OptionalRangeMappable {
  const OptionalRange({this.min, this.max});

  final int? min;
  final int? max;

  String format() => hiddifyRust.require(hiddifyRust.formatOptionalRange(min, max), 'formatOptionalRange');
  String present(TranslationsEn t) => format().isEmpty ? t.common.notSet : format();

  factory OptionalRange.parse(String input, {bool allowEmpty = false}) {
    final parsed = tryParse(input, allowEmpty: allowEmpty);
    if (parsed == null) throw Exception("Invalid range: $input");
    return parsed;
  }

  static OptionalRange? tryParse(String input, {bool allowEmpty = false}) {
    hiddifyRust.ensureAvailable('parseOptionalRange');
    final parsed = hiddifyRust.parseOptionalRange(input, allowEmpty: allowEmpty);
    if (parsed == null) return null;
    return OptionalRange(min: parsed.min, max: parsed.max);
  }
}

class OptionalRangeJsonConverter implements JsonConverter<OptionalRange, String> {
  const OptionalRangeJsonConverter();

  @override
  OptionalRange fromJson(String json) => OptionalRange.parse(json, allowEmpty: true);

  @override
  String toJson(OptionalRange object) => object.format();
}
