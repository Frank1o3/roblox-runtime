# Observed JNI surface

This is the initial behavior inventory for the Rust `jnivm` work. It was
extracted from one `rusty-blox --host-libc` session supplied with the runtime
workspace. That run reached Roblox's `APP_READY Home` notifications and
`gameLoadedCallback`, so it is useful evidence for the bootstrap and early game
surface. It is one client build and one run, not a complete JNI conformance
trace.

## Counts from the log

- 33 distinct classes reached through `FindClass`.
- 171 distinct class/member/signature records reported as found.
- 58 distinct Java methods called (after deduplicating repeated calls).
- 33 distinct field getter/setter signatures invoked.
- 13 distinct unresolved or unknown Java members (14 log records).
- Calls came from more than one native thread; `GetEnv` and
  `AttachCurrentThread` were both observed.

The `[JNIVM]` lines report class lookup, member resolution and Java-level
dispatch. They do not enumerate every `JNIEnv` function-table slot Roblox used,
so they guide class/member compatibility work but cannot alone define the
complete VM ABI needed for a Rust replacement.

## Classes reached

```text
android/app/ActivityThread
android/content/Context
android/content/res/Configuration
android/os/Build
android/os/Build$VERSION
android/os/LocaleList
android/view/KeyEvent
android/view/MotionEvent
androidx/core/graphics/Insets
androidx/core/view/WindowInsetsCompat$Type
com/google/androidgamesdk/GameActivity
com/google/androidgamesdk/gametextinput/InputConnection
com/google/androidgamesdk/gametextinput/State
com/roblox/audio/AppRtcDeviceWrapper
com/roblox/client/LocalStorageManager
com/roblox/client/flags/NativeFlagsInitResult
com/roblox/engine/jni/NativeGLJavaInterface
com/roblox/engine/jni/locale/NativeLocaleJavaInterface
com/roblox/engine/jni/memstorage/Connection
com/roblox/engine/jni/model/NativeTextBoxInfo
com/roblox/engine/jni/reporter/SessionReporterJavaInterface
com/roblox/engine/jni/user/NativeUserJavaInterface
com/roblox/engine/jni/util/NetworkUtils
com/roblox/engine/jni/video/MediaCodecInfoUtils
com/roblox/universalapp/logging/LoggingProtocol
com/roblox/universalapp/messagebus/Connection
com/snapchat/djinni/NativeObjectManager
java/lang/ClassLoader
java/util/Locale
org/fmod/AudioDevice
org/fmod/FMOD
org/fmod/MediaCodec
org/webrtc/voiceengine/WebRtcAudioManager
```

Additional classes appear in method/field records without a corresponding
`FindClass` line in this log, including `Application`, `SharedPreferences`,
`Resources`, `DisplayMetrics`, `DeviceStaticParams`, `PlatformParams`,
`InitParams`, `StartAppParams`, and `NativeHelper`.

## Unresolved or unknown operations

```text
java/lang/Class.getClassLoader()Ljava/lang/ClassLoader;                  (called unknown)
java/lang/Class.getClassLoader()Ljava/lang/ClassLoader;                  (unresolved member)
android/os/Build.MANUFACTURER:Ljava/lang/String;                         (unknown field getter)
android/os/Build.MANUFACTURER:Ljava/lang/String;                         (unresolved field)
com/google/androidgamesdk/GameActivity.finish()V
com/google/androidgamesdk/GameActivity.getWindowInsets(I)Landroidx/core/graphics/Insets;
com/roblox/engine/jni/NativeGLJavaInterface.getMobileAdvertisingId()V
com/roblox/engine/jni/NativeGLJavaInterface.onExtendedAnalyticsRecvCallback([BI)V
com/roblox/engine/jni/NativeGLJavaInterface.onVrSessionStateUpdate(I)V
com/roblox/engine/jni/NativeGLJavaInterface.promptNativePurchase(JLjava/lang/String;)V
com/roblox/engine/jni/NativeGLJavaInterface.promptNativePurchaseWithPaymentSessionId(JLjava/lang/String;Ljava/lang/String;)V
com/roblox/engine/jni/NativeGLJavaInterface.promptNativePurchaseWithPaymentSessionId(JLjava/lang/String;Ljava/lang/String;Ljava/lang/String;)V
com/roblox/engine/jni/NativeGLJavaInterface.saveImageToAlbum(Ljava/lang/String;)V
```

`getClassLoader` was reported unknown on both call records and was followed by
`ClassLoader.loadClass`/`findClass` activity. The log does not establish that
the unresolved operations are harmless in other flows; purchase, analytics,
VR, media and alternate startup paths need separate observations.

## Remaining observed compatibility work

The previous priorities for JavaVM attachment, member lookup, and string
handling now have initial Rust implementations. The unresolved operations above
still have no Rust handlers; placeholder method calls log the full member and
return descriptor-correct defaults. `java/lang/Class.getClassLoader` is the
first useful next hook because the recorded run followed it with class-loader
lookups. `android/os/Build.MANUFACTURER` also needs a field value. The
GameActivity and NativeGL callbacks need intentional behavior or a documented
no-op based on fresh runtime observations.

The available log is still the earlier `rusty-blox --host-libc` run described
above. No newer runtime log was present in the workspace during the latest
implementation pass.

The runtime must keep using the C++ backend until these JNI semantics and the
runtime's C++-registered Java compatibility classes have Rust equivalents.

## Rust crate status

The Rust modules include JNI and JavaVM ABI tables, per-thread attachment,
class/method/field lookup, native registration, local/global references,
strings, and method dispatch. The `A` method-call entries decode arguments by
descriptor. The `V` call entries still use descriptor-correct zero/null
arguments because the platform `va_list` is not decoded. Java compatibility
handlers are incomplete, and the Rust backend remains opt-in through
`USE_EXPERIMENTAL_JNIVM=true`.

When that backend is selected, Roblox receives the Rust JavaVM. The runtime also
starts a separate C++ libjnivm VM for Cordial's existing compatibility classes,
which runtime-owned direct calls use during startup and app-bridge setup. Those
classes have not yet been ported to Rust, so this companion VM is part of the
current experimental arrangement.
