part of 'engine.dart';

/// Owns one native EPUB book handle under a [PageletEngine].
final class BookSession {
  BookSession._(this._engine, this._handle);

  final PageletEngine _engine;
  final int _handle;
  bool _isDisposed = false;

  /// Whether this book was disposed directly or by its owning engine.
  bool get isDisposed => _isDisposed;

  /// Disposes the native book and all of its descendant handles.
  ///
  /// Repeated calls are safe. Disposing the owning engine also invalidates the
  /// session without issuing a redundant native call.
  void dispose() {
    _engine._disposeBook(this);
  }

  void _markDisposedByOwner() {
    _isDisposed = true;
  }
}
