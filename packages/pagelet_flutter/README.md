# pagelet_flutter

Flutter host bindings for the native `pagelet` EPUB parsing and pagination
engine.

The adapter owns the engine and book-session lifecycle, provides batched
Flutter paragraph measurement, decodes versioned page scenes into Flutter-ready
geometry, and copies publication resources into Dart memory on demand.
Opened books expose typed metadata, spine order, navigation, and diagnostics
for host-side shadow-parser comparisons.

The [`example/`](example/) reader connects the native chapter/layout lifecycle,
Flutter measurement, page-scene decoding, and responsive `PageView` painting.

`dart tool/build_smoke.dart macos` builds a temporary host app. CI runs the same
smoke for Android, iOS, macOS, and Windows without committing generated runner
projects.

```dart
final engine = PageletEngine(libraryPath: '/path/to/libpagelet.dylib');
final book = engine.openBook('/path/to/book.epub');
final summary = book.summary;
final cover = book.resources.read(coverResourceId);

try {
  final measurement = TextMeasureBridge(
    fontFingerprint: myRegisteredFontSetFingerprint,
  ).measureBatch(nativeMeasureBatchBytes);
  try {
    final page = const PageSceneDecoder()
        .decode(nativePageBatchBytes)
        .pages
        .first;
    // Paint page.textPaints with the matching retained TextPainters.
  } finally {
    measurement.dispose();
  }
} finally {
  book.dispose();
  engine.dispose();
}
```

If `libraryPath` is omitted, the package loads the platform library name:
`libpagelet.dylib`, `libpagelet.so`, or `pagelet.dll`. iOS resolves statically
linked symbols from the current process.
