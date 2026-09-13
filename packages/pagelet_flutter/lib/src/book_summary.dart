import 'dart:convert';
import 'dart:typed_data';

/// Metadata, reading order, navigation, and diagnostics for an opened EPUB.
final class PageletBookSummary {
  const PageletBookSummary({
    required this.rootfile,
    required this.packageVersion,
    required this.identifier,
    required this.title,
    required this.language,
    required this.spine,
    required this.navigation,
    required this.diagnostics,
  });

  final String rootfile;
  final String packageVersion;
  final String? identifier;
  final String? title;
  final String? language;
  final List<PageletSpineItem> spine;
  final PageletNavigation navigation;
  final List<PageletDiagnostic> diagnostics;

  factory PageletBookSummary.decode(Uint8List bytes) {
    final root = _object(jsonDecode(utf8.decode(bytes)), 'book summary');
    return PageletBookSummary(
      rootfile: _string(root, 'rootfile'),
      packageVersion: _string(root, 'package_version'),
      identifier: _optionalString(root, 'identifier'),
      title: _optionalString(root, 'title'),
      language: _optionalString(root, 'language'),
      spine: List<PageletSpineItem>.unmodifiable(
        _array(root, 'spine').map(
          (value) => PageletSpineItem._decode(_object(value, 'spine item')),
        ),
      ),
      navigation: PageletNavigation._decode(
        _object(root['navigation'], 'navigation'),
      ),
      diagnostics: List<PageletDiagnostic>.unmodifiable(
        _array(root, 'diagnostics').map(
          (value) => PageletDiagnostic._decode(
            _object(value, 'diagnostic'),
          ),
        ),
      ),
    );
  }
}

final class PageletSpineItem {
  const PageletSpineItem({required this.idref, required this.linear});

  final String idref;
  final bool linear;

  factory PageletSpineItem._decode(Map<String, Object?> value) {
    return PageletSpineItem(
      idref: _string(value, 'idref'),
      linear: _boolean(value, 'linear'),
    );
  }
}

final class PageletNavigation {
  const PageletNavigation({
    required this.source,
    required this.toc,
    required this.pageList,
    required this.landmarks,
  });

  final String source;
  final List<PageletNavigationItem> toc;
  final List<PageletNavigationItem> pageList;
  final List<PageletNavigationItem> landmarks;

  factory PageletNavigation._decode(Map<String, Object?> value) {
    List<PageletNavigationItem> items(String key) {
      return List<PageletNavigationItem>.unmodifiable(
        _array(value, key).map(
          (item) => PageletNavigationItem._decode(
            _object(item, 'navigation item'),
          ),
        ),
      );
    }

    return PageletNavigation(
      source: _string(value, 'source'),
      toc: items('toc'),
      pageList: items('page_list'),
      landmarks: items('landmarks'),
    );
  }
}

final class PageletNavigationItem {
  const PageletNavigationItem({
    required this.label,
    required this.href,
    required this.children,
  });

  final String label;
  final String href;
  final List<PageletNavigationItem> children;

  factory PageletNavigationItem._decode(Map<String, Object?> value) {
    return PageletNavigationItem(
      label: _string(value, 'label'),
      href: _string(value, 'href'),
      children: List<PageletNavigationItem>.unmodifiable(
        _array(value, 'children').map(
          (item) => PageletNavigationItem._decode(
            _object(item, 'navigation child'),
          ),
        ),
      ),
    );
  }
}

final class PageletDiagnostic {
  const PageletDiagnostic({
    required this.code,
    required this.severity,
    required this.message,
  });

  final String code;
  final String severity;
  final String message;

  factory PageletDiagnostic._decode(Map<String, Object?> value) {
    return PageletDiagnostic(
      code: _string(value, 'code'),
      severity: _string(value, 'severity'),
      message: _string(value, 'message'),
    );
  }
}

Map<String, Object?> _object(Object? value, String field) {
  if (value is Map<String, dynamic>) {
    return Map<String, Object?>.from(value);
  }
  throw FormatException('$field must be a JSON object.');
}

List<Object?> _array(Map<String, Object?> value, String key) {
  final field = value[key];
  if (field is List) {
    return List<Object?>.from(field);
  }
  throw FormatException('$key must be a JSON array.');
}

String _string(Map<String, Object?> value, String key) {
  final field = value[key];
  if (field is String) {
    return field;
  }
  throw FormatException('$key must be a string.');
}

String? _optionalString(Map<String, Object?> value, String key) {
  final field = value[key];
  if (field == null || field is String) {
    return field as String?;
  }
  throw FormatException('$key must be a string or null.');
}

bool _boolean(Map<String, Object?> value, String key) {
  final field = value[key];
  if (field is bool) {
    return field;
  }
  throw FormatException('$key must be a boolean.');
}
