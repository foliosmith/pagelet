part of 'engine.dart';

/// Owns one native chapter under a [BookSession].
final class ChapterSession {
  ChapterSession._(this._book, this._handle);

  final BookSession _book;
  final int _handle;
  final Set<LayoutSession> _layouts = <LayoutSession>{};
  bool _isDisposed = false;

  bool get isDisposed => _isDisposed;

  LayoutSession createLayout(PageletLayoutOptions options) {
    _ensureOpen();
    final handle = _requireHandle(
      _book._engine._nativeApi,
      _book._engine._nativeApi.layoutSessionCreate(
        _handle,
        _nativeLayoutOptions(options),
      ),
      'layout_session_create',
    );
    final layout = LayoutSession._(this, handle);
    _layouts.add(layout);
    return layout;
  }

  void dispose() => _book._disposeChapter(this);

  void _disposeLayout(LayoutSession layout) {
    if (layout._isDisposed) {
      return;
    }
    if (_isDisposed) {
      layout._markDisposedByOwner();
      return;
    }
    _requireSuccess(
      _book._engine._nativeApi.handleDispose(layout._handle),
      'handle_dispose(layout)',
    );
    layout._markDisposedByOwner();
    _layouts.remove(layout);
  }

  void _markDisposedByOwner() {
    _isDisposed = true;
    for (final layout in _layouts.toList(growable: false)) {
      layout._markDisposedByOwner();
    }
    _layouts.clear();
  }

  void _ensureOpen() {
    if (_isDisposed) {
      throw StateError('ChapterSession has been disposed.');
    }
  }
}
