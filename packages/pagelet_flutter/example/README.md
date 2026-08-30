# pagelet Flutter reader example

This minimal reader opens the first spine item, measures text through Flutter,
decodes the returned page scenes, and paints up to three pages in a `PageView`.

Run it from this directory after adding the Flutter platform you need:

```sh
flutter create --platforms=macos .
flutter run -d macos \
  --dart-define=PAGELET_LIBRARY_PATH=/path/to/libpagelet.dylib \
  --dart-define=PAGELET_BOOK_PATH=/path/to/book.epub
```

Platform scaffolding is intentionally left to Flutter so the example does not
duplicate generated runner files. Multi-platform build coverage belongs to
Task 4.6.5.
