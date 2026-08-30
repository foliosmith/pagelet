import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:pagelet_flutter/pagelet_flutter.dart';

void main() {
  const decoder = PageSceneDecoder();

  test('decodes the fixed empty v3 page batch', () {
    final batch = decoder.decode(
      Uint8List.fromList(const <int>[
        0x50,
        0x47,
        0x4c,
        0x54,
        0x53,
        0x43,
        0x4e,
        0x00,
        0x03,
        0x00,
        0x01,
        0x00,
        0x04,
        0x00,
        0x00,
        0x00,
        0x1c,
        0xdf,
        0x44,
        0x21,
        0x00,
        0x00,
        0x00,
        0x00,
      ]),
    );

    expect(batch.schemaVersion, 3);
    expect(batch.pages, isEmpty);
  });

  test('rejects a non-page payload and checksum corruption', () {
    final wrongKind = Uint8List.fromList(const <int>[
      0x50,
      0x47,
      0x4c,
      0x54,
      0x53,
      0x43,
      0x4e,
      0x00,
      0x03,
      0x00,
      0x02,
      0x00,
      0x04,
      0x00,
      0x00,
      0x00,
      0x1c,
      0xdf,
      0x44,
      0x21,
      0x00,
      0x00,
      0x00,
      0x00,
    ]);
    final corrupt = Uint8List.fromList(wrongKind)
      ..[10] = 1
      ..[23] = 1;

    expect(() => decoder.decode(wrongKind), throwsFormatException);
    expect(() => decoder.decode(corrupt), throwsFormatException);
  });
}
