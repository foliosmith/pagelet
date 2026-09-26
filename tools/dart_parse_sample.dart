import 'dart:convert';
import 'dart:io';

import 'package:epub_reader_core/epub_reader_core.dart';

void main(List<String> arguments) {
  if (arguments.length != 1) {
    throw ArgumentError('dart_parse_sample <epub>');
  }
  final bytes = File(arguments.single).readAsBytesSync();
  final timer = Stopwatch()..start();
  final book = EpubParser.parseBytesSync(bytes);
  final visibleChars = book.chapters.fold<int>(
    0,
    (count, chapter) => count + chapter.plainText.runes.length,
  );
  timer.stop();
  stdout.writeln(
    jsonEncode({
      'parse_ns': timer.elapsedTicks * 1000000000 ~/ timer.frequency,
      'chapters': book.chapters.length,
      'visible_chars': visibleChars,
    }),
  );
}
