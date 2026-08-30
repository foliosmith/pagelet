import 'dart:io';

Future<void> main(List<String> arguments) async {
  if (arguments.length != 1 ||
      !const <String>{'android', 'ios', 'macos', 'windows'}
          .contains(arguments.single)) {
    stderr
        .writeln('usage: dart tool/build_smoke.dart android|ios|macos|windows');
    exitCode = 2;
    return;
  }

  final target = arguments.single;
  final packageDirectory = File.fromUri(Platform.script).parent.parent.absolute;
  final smokeDirectory = Directory.systemTemp.createTempSync(
    'pagelet-flutter-$target.',
  );
  try {
    await _runFlutter(<String>[
      '--no-version-check',
      'create',
      '--no-pub',
      '--platforms=$target',
      '--project-name=pagelet_flutter_smoke',
      smokeDirectory.path,
    ]);
    File('${packageDirectory.path}/tool/smoke_main.dart').copySync(
      '${smokeDirectory.path}/lib/main.dart',
    );
    await _runFlutter(
      <String>[
        '--no-version-check',
        'pub',
        'add',
        'pagelet_flutter@{path: ${packageDirectory.path}}',
      ],
      workingDirectory: smokeDirectory.path,
    );
    await _runFlutter(
      <String>[
        '--no-version-check',
        'build',
        ...switch (target) {
          'android' => <String>['apk', '--debug'],
          'ios' => <String>['ios', '--debug', '--simulator'],
          'macos' => <String>['macos', '--debug'],
          'windows' => <String>['windows', '--debug'],
          _ => throw StateError('validated target became invalid'),
        },
      ],
      workingDirectory: smokeDirectory.path,
    );
  } finally {
    smokeDirectory.deleteSync(recursive: true);
  }
}

Future<void> _runFlutter(
  List<String> arguments, {
  String? workingDirectory,
}) async {
  final process = await Process.start(
    'flutter',
    arguments,
    workingDirectory: workingDirectory,
    mode: ProcessStartMode.inheritStdio,
    runInShell: Platform.isWindows,
  );
  final result = await process.exitCode;
  if (result != 0) {
    throw ProcessException('flutter', arguments, 'exit code $result', result);
  }
}
