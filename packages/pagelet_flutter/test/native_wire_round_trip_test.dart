import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:pagelet_flutter/pagelet_flutter.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  final requestPath = Platform.environment['PAGELET_MEASURE_REQUEST_PATH'];
  final responsePath = Platform.environment['PAGELET_MEASURE_RESPONSE_PATH'];

  test(
    'measures a Rust-encoded batch for Rust-side response verification',
    () {
      final measurement = const TextMeasureBridge(
        fontFingerprint: 0x12345678,
      ).measureBatch(File(requestPath!).readAsBytesSync());
      try {
        expect(measurement.paragraphs.keys, <int>{70, 80});
        File(responsePath!).writeAsBytesSync(measurement.wireBytes);
      } finally {
        measurement.dispose();
      }
    },
    skip: requestPath == null || responsePath == null
        ? 'Set PAGELET_MEASURE_REQUEST_PATH and PAGELET_MEASURE_RESPONSE_PATH.'
        : false,
  );
}
