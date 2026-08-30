import 'dart:convert';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:pagelet_flutter/pagelet_flutter.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  group('TextMeasureBridge', () {
    test('measures a complete multi-paragraph batch in one call', () {
      const bridge = TextMeasureBridge(fontFingerprint: 0x12345678);
      final request = _encodeMeasureBatch(<_Request>[
        const _Request(
          id: 7,
          paragraphId: 70,
          text: 'Hello 中🙂\nsecond line',
          width: 120,
          fingerprint: 0x0707,
        ),
        const _Request(
          id: 8,
          paragraphId: 80,
          text: 'مرحبا بالعالم',
          width: 80,
          direction: 2,
          fingerprint: 0x0808,
        ),
      ]);

      final measurement = bridge.measureBatch(request);
      addTearDown(measurement.dispose);

      expect(measurement.paragraphs.keys, <int>{70, 80});
      expect(measurement.paragraphs[70]!.text, 'Hello 中🙂\nsecond line');
      expect(
          measurement.paragraphs[70]!.painter.computeLineMetrics(), isNotEmpty);
      expect(
          measurement.paragraphs[80]!.painter.computeLineMetrics(), isNotEmpty);

      final response = _decodeMeasuredBatch(measurement.wireBytes);
      expect(response.schemaVersion, 3);
      expect(response.kind, 3);
      expect(response.backendId, pageletFlutterTextBackendId);
      expect(response.fontFingerprint, 0x12345678);
      expect(response.results, hasLength(2));
      expect(response.results.map((result) => result.requestId), <int>[7, 8]);
      expect(response.results[0].requestFingerprint, 0x0707);
      expect(response.results[0].utf8Length,
          utf8.encode('Hello 中🙂\nsecond line').length);
      expect(response.results[0].lineCount, greaterThanOrEqualTo(2));
      expect(response.results[0].hasHardBreak, isTrue);
      expect(
        response.results[0].clusterRanges,
        contains((6, 9)),
        reason: 'CJK offsets must be UTF-8 byte offsets',
      );
      expect(
        response.results[0].clusterRanges,
        contains((9, 13)),
        reason: 'emoji offsets must be UTF-8 byte offsets',
      );
      expect(response.results[0].measurementFingerprint, isNot(0));

      final callerCopy = measurement.wireBytes;
      callerCopy[0] = 0;
      expect(measurement.wireBytes[0], _magic.first);
    });

    test('retains the exact painters until the result is disposed', () {
      const bridge = TextMeasureBridge(fontFingerprint: 9);
      final measurement = bridge.measureBatch(
        _encodeMeasureBatch(const <_Request>[
          _Request(
            id: 1,
            paragraphId: 10,
            text: 'retained paragraph',
            width: 100,
            fingerprint: 11,
          ),
        ]),
      );
      final paragraph = measurement.paragraphs[10]!;

      expect(paragraph.isDisposed, isFalse);
      expect(paragraph.painter.width, greaterThan(0));

      measurement.dispose();
      measurement.dispose();

      expect(measurement.isDisposed, isTrue);
      expect(paragraph.isDisposed, isTrue);
    });

    test('rejects corrupted and structurally invalid batches', () {
      const bridge = TextMeasureBridge(fontFingerprint: 1);
      final corrupted = _encodeMeasureBatch(const <_Request>[
        _Request(
          id: 1,
          paragraphId: 10,
          text: 'checksum',
          width: 100,
          fingerprint: 1,
        ),
      ]);
      corrupted[corrupted.length - 1] ^= 0xff;

      expect(() => bridge.measureBatch(corrupted), throwsFormatException);
      expect(
        () => bridge.measureBatch(
          _encodeMeasureBatch(const <_Request>[
            _Request(
              id: 1,
              paragraphId: 10,
              text: 'first',
              width: 100,
              fingerprint: 1,
            ),
            _Request(
              id: 2,
              paragraphId: 10,
              text: 'duplicate paragraph',
              width: 100,
              fingerprint: 2,
            ),
          ]),
        ),
        throwsFormatException,
      );

      final invalidUtf8Boundary = _encodeMeasureBatch(const <_Request>[
        _Request(
          id: 3,
          paragraphId: 30,
          text: '中',
          width: 100,
          fingerprint: 3,
        ),
      ]);
      final payload = Uint8List.sublistView(invalidUtf8Boundary, 20);
      final payloadData = ByteData.sublistView(payload);
      payloadData.setUint32(24, 1, Endian.little);
      ByteData.sublistView(invalidUtf8Boundary, 0, 20).setUint32(
        16,
        _crc32(payload),
        Endian.little,
      );
      expect(
        () => bridge.measureBatch(invalidUtf8Boundary),
        throwsFormatException,
      );
    });
  });
}

const int _layoutScale = 64;
const List<int> _magic = <int>[
  0x50,
  0x47,
  0x4c,
  0x54,
  0x53,
  0x43,
  0x4e,
  0,
];

final class _Request {
  const _Request({
    required this.id,
    required this.paragraphId,
    required this.text,
    required this.width,
    required this.fingerprint,
    this.direction = 1,
  });

  final int id;
  final int paragraphId;
  final String text;
  final int width;
  final int fingerprint;
  final int direction;
}

Uint8List _encodeMeasureBatch(List<_Request> requests) {
  final writer = _Writer()..u32(requests.length);
  for (final request in requests) {
    final textLength = utf8.encode(request.text).length;
    writer
      ..u32(request.id)
      ..u32(request.paragraphId)
      ..string(request.text)
      ..u32(0)
      ..u32(textLength)
      ..u32(1)
      ..u32(0)
      ..u32(textLength)
      ..i64(16 * _layoutScale)
      ..i64(0)
      ..font('Ahem')
      ..i64(16 * _layoutScale)
      ..i64(request.width * _layoutScale)
      ..i64(request.width * _layoutScale)
      ..string('en-US')
      ..u8(request.direction)
      ..i64(_layoutScale)
      ..font('Ahem')
      ..i64(0)
      ..i64(0)
      ..i64(0)
      ..u8(0)
      ..u64(request.fingerprint);
  }
  return _envelope(3, 2, writer.take());
}

_MeasuredResponse _decodeMeasuredBatch(Uint8List bytes) {
  final data = ByteData.sublistView(bytes);
  expect(bytes.sublist(0, 8), _magic);
  final schemaVersion = data.getUint16(8, Endian.little);
  final kind = data.getUint16(10, Endian.little);
  final payloadLength = data.getUint32(12, Endian.little);
  final payload = Uint8List.sublistView(bytes, 20);
  expect(payload.length, payloadLength);
  expect(_crc32(payload), data.getUint32(16, Endian.little));
  final reader = _Reader(payload);
  final backendId = reader.u64();
  final fontFingerprint = reader.u64();
  final count = reader.u32();
  final results = <_MeasuredResult>[];
  for (var resultIndex = 0; resultIndex < count; resultIndex += 1) {
    final requestId = reader.u32();
    final requestFingerprint = reader.u64();
    reader
      ..i64()
      ..i64();
    final declaredLineCount = reader.u32();
    final utf8Length = reader.u32();
    final lineCount = reader.u32();
    expect(declaredLineCount, lineCount);
    var hasHardBreak = false;
    for (var lineIndex = 0; lineIndex < lineCount; lineIndex += 1) {
      reader
        ..u32()
        ..u32()
        ..i64()
        ..i64()
        ..i64()
        ..i64()
        ..i64()
        ..i64()
        ..i64()
        ..i64()
        ..i64();
      hasHardBreak |= reader.u8() == 1;
    }
    final clusterCount = reader.u32();
    final clusterRanges = <(int, int)>[];
    for (var clusterIndex = 0; clusterIndex < clusterCount; clusterIndex += 1) {
      clusterRanges.add((reader.u32(), reader.u32()));
      reader
        ..u32()
        ..i64()
        ..i64();
    }
    results.add(
      _MeasuredResult(
        requestId: requestId,
        requestFingerprint: requestFingerprint,
        utf8Length: utf8Length,
        lineCount: lineCount,
        hasHardBreak: hasHardBreak,
        clusterRanges: clusterRanges,
        measurementFingerprint: reader.u64(),
      ),
    );
  }
  expect(reader.offset, payload.length);
  return _MeasuredResponse(
    schemaVersion: schemaVersion,
    kind: kind,
    backendId: backendId,
    fontFingerprint: fontFingerprint,
    results: results,
  );
}

final class _MeasuredResponse {
  const _MeasuredResponse({
    required this.schemaVersion,
    required this.kind,
    required this.backendId,
    required this.fontFingerprint,
    required this.results,
  });

  final int schemaVersion;
  final int kind;
  final int backendId;
  final int fontFingerprint;
  final List<_MeasuredResult> results;
}

final class _MeasuredResult {
  const _MeasuredResult({
    required this.requestId,
    required this.requestFingerprint,
    required this.utf8Length,
    required this.lineCount,
    required this.hasHardBreak,
    required this.clusterRanges,
    required this.measurementFingerprint,
  });

  final int requestId;
  final int requestFingerprint;
  final int utf8Length;
  final int lineCount;
  final bool hasHardBreak;
  final List<(int, int)> clusterRanges;
  final int measurementFingerprint;
}

final class _Writer {
  final BytesBuilder _bytes = BytesBuilder(copy: false);

  void u8(int value) => _bytes.addByte(value);

  void u16(int value) {
    final data = ByteData(2)..setUint16(0, value, Endian.little);
    _bytes.add(data.buffer.asUint8List());
  }

  void u32(int value) {
    final data = ByteData(4)..setUint32(0, value, Endian.little);
    _bytes.add(data.buffer.asUint8List());
  }

  void u64(int value) {
    final data = ByteData(8)..setUint64(0, value, Endian.little);
    _bytes.add(data.buffer.asUint8List());
  }

  void i64(int value) {
    final data = ByteData(8)..setInt64(0, value, Endian.little);
    _bytes.add(data.buffer.asUint8List());
  }

  void string(String value) {
    final bytes = utf8.encode(value);
    u32(bytes.length);
    _bytes.add(bytes);
  }

  void font(String family) {
    u8(0);
    string(family);
    u16(400);
    u8(0);
    u16(100);
    u64(0);
    u32(0);
  }

  Uint8List take() => _bytes.takeBytes();
}

final class _Reader {
  _Reader(Uint8List bytes) : _data = ByteData.sublistView(bytes);

  final ByteData _data;
  int offset = 0;

  int u8() {
    final value = _data.getUint8(offset);
    offset += 1;
    return value;
  }

  int u32() {
    final value = _data.getUint32(offset, Endian.little);
    offset += 4;
    return value;
  }

  int u64() {
    final value = _data.getUint64(offset, Endian.little);
    offset += 8;
    return value;
  }

  int i64() {
    final value = _data.getInt64(offset, Endian.little);
    offset += 8;
    return value;
  }
}

Uint8List _envelope(int version, int kind, Uint8List payload) {
  final bytes = Uint8List(20 + payload.length)..setAll(0, _magic);
  final header = ByteData.sublistView(bytes, 0, 20);
  header
    ..setUint16(8, version, Endian.little)
    ..setUint16(10, kind, Endian.little)
    ..setUint32(12, payload.length, Endian.little)
    ..setUint32(16, _crc32(payload), Endian.little);
  bytes.setAll(20, payload);
  return bytes;
}

int _crc32(Uint8List bytes) {
  var crc = 0xffffffff;
  for (final byte in bytes) {
    crc ^= byte;
    for (var bit = 0; bit < 8; bit += 1) {
      crc = (crc & 1) == 1 ? (crc >> 1) ^ 0xedb88320 : crc >> 1;
    }
  }
  return (crc ^ 0xffffffff) & 0xffffffff;
}
