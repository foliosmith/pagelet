import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:pagelet_flutter/pagelet_flutter.dart';

void main() {
  final libraryPath = Platform.environment['PAGELET_LIBRARY_PATH'];

  test(
    'creates and disposes an engine through the real C ABI',
    () {
      final engine = PageletEngine(libraryPath: libraryPath!);
      try {
        expect(engine.isDisposed, isFalse);
        expect(
          () => engine.openBook(
            '/pagelet-flutter-native-smoke/missing.epub',
          ),
          throwsA(
            isA<PageletException>().having(
              (error) => error.status,
              'status',
              PageletStatus.io,
            ),
          ),
        );
      } finally {
        engine.dispose();
      }
      expect(engine.isDisposed, isTrue);
    },
    skip: libraryPath == null
        ? 'Set PAGELET_LIBRARY_PATH to run the native ABI smoke test.'
        : false,
  );
}
