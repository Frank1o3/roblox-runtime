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

## Initial implementation priorities

1. Match the current JavaVM invocation behavior, including per-thread env
   attachment and the observed worker-thread calls.
2. Implement JNI references, class/method/field lookup and dispatch for the
   observed entries, using the existing descriptor parser in this crate.
3. Cover strings, object and primitive arrays, and `ByteBuffer`; those types
   occur in observed signatures and bootstrap callbacks.
4. Re-run logging on startup, login, settings, audio, text input, and a game
   session before treating the inventory as sufficient to select the Rust VM.

The runtime must keep using the C++ backend until these JNI semantics and the
runtime's C++-registered Java compatibility classes have Rust equivalents.

## Rust crate status

The initial Rust modules now include a method/field descriptor parser and a
VM state model for per-thread attachment, class lookup/placeholder creation,
method and field IDs, native registrations, and local/global object references.
These are internal building blocks only: there is not yet a JNI C ABI function
table, JavaVM invocation table, Java class behavior, or runtime backend switch.
