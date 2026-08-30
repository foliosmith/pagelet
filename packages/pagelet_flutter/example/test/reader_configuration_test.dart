import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import '../lib/reader_configuration.dart';

void main() {
  const base = PageletReaderConfiguration();

  test('classifies paint, repack, and reflow changes', () {
    expect(
      const PageletReaderConfiguration(textColor: Colors.blue).impactFrom(base),
      ReaderLayoutImpact.paintOnly,
    );
    expect(
      const PageletReaderConfiguration(viewportHeight: 700).impactFrom(base),
      ReaderLayoutImpact.repackOnly,
    );
    expect(
      const PageletReaderConfiguration(marginTop: 12).impactFrom(base),
      ReaderLayoutImpact.repackOnly,
    );
    expect(
      const PageletReaderConfiguration(viewportWidth: 500).impactFrom(base),
      ReaderLayoutImpact.reflow,
    );
    expect(
      const PageletReaderConfiguration(fontFingerprint: 1).impactFrom(base),
      ReaderLayoutImpact.reflow,
    );
    expect(
      const PageletReaderConfiguration(letterSpacing: 1).impactFrom(base),
      ReaderLayoutImpact.reflow,
    );
  });
}
