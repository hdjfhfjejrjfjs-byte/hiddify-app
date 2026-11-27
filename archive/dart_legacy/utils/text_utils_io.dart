// ARCHIVED LEGACY DART IMPLEMENTATION
// Original path: lib/utils/text_utils.dart (decodeTextBytes & readZipText)
// Replaced by:   rust/crates/hiddify-profile (decode_text_bytes, read_zip_text)

import 'dart:convert';
import 'dart:typed_data';

import 'package:archive/archive.dart' show Archive, ZipDecoder;

const _maxZipFileBytes = 256 * 1024;
const zipTextSeparator = '\n#===\n';

String legacyDecodeTextBytes(Uint8List bytes) {
  if (bytes.length >= 3 && bytes[0] == 0xEF && bytes[1] == 0xBB && bytes[2] == 0xBF) {
    return utf8.decode(bytes.sublist(3), allowMalformed: true);
  }
  return utf8.decode(bytes, allowMalformed: true);
}

String legacyReadZipText(Uint8List bytes) {
  final Archive archive;
  try {
    archive = ZipDecoder().decodeBytes(bytes);
  } catch (_) {
    return '';
  }
  final texts = <String>[];
  for (final file in archive) {
    final segments = file.name.replaceAll('\\', '/').split('/');
    final fileName = segments.last;
    if (!file.isFile || segments.contains('__MACOSX') || fileName.startsWith('._') || file.size > _maxZipFileBytes) {
      continue;
    }
    try {
      texts.add(legacyDecodeTextBytes(file.readBytes() ?? Uint8List(0)));
    } catch (_) {
      // A damaged or password-protected file must not stop the others from being read.
    }
  }
  return texts.join(zipTextSeparator);
}
