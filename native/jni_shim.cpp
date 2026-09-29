// extern "C" surface over libjnivm, so Rust can stand up a JavaVM.
//
// As with shim.cpp: translation only, no policy.

#include <jnivm.h>

extern "C" void roblox_register_android_classes(void* env);
namespace roblox_runtime {
void register_game_activity_classes(jnivm::ENV* env);
void register_init_params_classes(jnivm::ENV* env);
}

/// The process VM, for translation units that need the real jnivm::ENV rather
/// than a JNIEnv. Keeping this on the C++ side means Rust never has to hold a
/// type it cannot name — passing a JNIEnv* and casting it to jnivm::ENV* is a
/// silent type confusion that surfaces as a null function pointer several calls
/// later.
namespace roblox_runtime { jnivm::ENV* process_env(); }

#include <cstddef>
#include <cstdio>
#include <cstdlib>
#include <exception>
#include <execinfo.h>
#include <unistd.h>
#include <stdexcept>
#include <memory>
#include <algorithm>
#include <cstdint>
#include <cstring>

namespace {
// The VM owns every class, object and environment it hands out, so it outlives
// anything derived from it. One per process.
std::unique_ptr<jnivm::VM> g_vm;
const JNIInvokeInterface* g_real_iface = nullptr;
JavaVM* g_real_vm = nullptr;

/// Stand up libjnivm's compatibility classes independently of the JavaVM
/// Roblox receives. The experimental Rust VM owns that JavaVM, while existing
/// Cordial Java hooks still use libjnivm objects when the runtime invokes
/// exported engine natives directly (GameActivity/app-bridge startup).
bool initialize_compat_vm() {
    if (g_vm) {
        return true;
    }
    g_vm = std::make_unique<jnivm::VM>();
    roblox_register_android_classes(g_vm->GetEnv().get());
    roblox_runtime::register_game_activity_classes(g_vm->GetEnv().get());
    roblox_runtime::register_init_params_classes(g_vm->GetEnv().get());
    g_real_vm = g_vm->GetJavaVM();
    g_real_iface = g_real_vm->functions;
    return true;
}

/// Report an uncaught C++ exception with its stack instead of dying mute.
///
/// Roblox spawns worker threads during `JNI_OnLoad` and they call straight back
/// into JNI. libjnivm reports misuse by throwing, and an exception escaping a
/// thread Cordial did not start cannot be caught anywhere — the default is
/// `std::terminate` and a core dump carrying no information about which thread,
/// which call, or why.
/// Write the observed Java surface out, if a VM exists and a path was given.
void dump_classes_now() {
#ifdef JNI_DEBUG
    const char* path = getenv("RBX_RUNTIME_JNI_DUMP");
    if (path && g_vm) {
        try {
            g_vm->GenerateClassDump(path);
            fprintf(stderr, "    Java surface Roblox reached for -> %s\n", path);
        } catch (...) {
            fprintf(stderr, "    class dump failed\n");
        }
    }
#endif
}

[[noreturn]] void report_terminate() {
    fprintf(stderr, "\n*** uncaught C++ exception on thread %ld ***\n", (long)gettid());
    if (auto e = std::current_exception()) {
        try {
            std::rethrow_exception(e);
        } catch (const std::exception& ex) {
            fprintf(stderr, "    what(): %s\n", ex.what());
        } catch (...) {
            fprintf(stderr, "    non-standard exception\n");
        }
    }
    void* frames[32];
    int n = backtrace(frames, 32);
    fprintf(stderr, "    %d frames:\n", n);
    backtrace_symbols_fd(frames, n, 2);

    // Dump before dying. Everything Roblox asked Java for up to this point is
    // the most valuable thing in the process, and it is about to be lost — the
    // failure happens on a thread nobody can catch, so there is no later.
    dump_classes_now();
    _exit(70);
}
} // namespace

namespace {

// A JavaVM whose invocation table logs before delegating. Roblox died before
// asking for a single Java class, so the failure is in the invocation interface
// itself — which of DestroyJavaVM / AttachCurrentThread / DetachCurrentThread /
// GetEnv, called with which JavaVM, was not visible any other way.
JNIInvokeInterface g_traced_iface;
JavaVM g_traced_vm;
bool g_trace_invoke = false;

void note(const char* what, JavaVM* vm) {
    if (g_trace_invoke) {
        fprintf(stderr, "[jni] %s(vm=%p)%s\n", what, (void*)vm,
                vm == &g_traced_vm ? " [ours]" : " [NOT ours]");
    }
}

jint traced_destroy(JavaVM* vm) {
    note("DestroyJavaVM", vm);
    return g_real_iface->DestroyJavaVM(g_real_vm);
}
jint traced_attach(JavaVM* vm, JNIEnv** env, void* args) {
    note("AttachCurrentThread", vm);
    return g_real_iface->AttachCurrentThread(g_real_vm, env, args);
}
jint traced_attach_daemon(JavaVM* vm, JNIEnv** env, void* args) {
    note("AttachCurrentThreadAsDaemon", vm);
    return g_real_iface->AttachCurrentThreadAsDaemon(g_real_vm, env, args);
}
jint traced_detach(JavaVM* vm) {
    note("DetachCurrentThread", vm);
    return g_real_iface->DetachCurrentThread(g_real_vm);
}
jint traced_get_env(JavaVM* vm, void** env, jint version) {
    note("GetEnv", vm);
    return g_real_iface->GetEnv(g_real_vm, env, version);
}

} // namespace

extern "C" {

/// Create the process's JavaVM. Returns the `JavaVM*` Roblox expects in
/// `JNI_OnLoad`, or null if one already exists.
void* roblox_jni_create_vm() {
    if (g_vm) {
        return nullptr;
    }
    std::set_terminate(report_terminate);
    initialize_compat_vm();

    // Copy the table wholesale — reserved0 has to survive, or libjnivm cannot
    // recover its own VM — then replace only the entry points.
    g_traced_iface = *g_real_iface;
    g_traced_iface.DestroyJavaVM = traced_destroy;
    g_traced_iface.AttachCurrentThread = traced_attach;
    g_traced_iface.AttachCurrentThreadAsDaemon = traced_attach_daemon;
    g_traced_iface.DetachCurrentThread = traced_detach;
    g_traced_iface.GetEnv = traced_get_env;
    g_traced_vm.functions = &g_traced_iface;
    g_trace_invoke = getenv("RBX_RUNTIME_JNI_TRACE") != nullptr;

    JavaVM* vm = &g_traced_vm;
    // libjnivm recovers its VM from JavaVM::functions->reserved0. If that is not
    // set, every callback Roblox makes throws before it does anything.
    fprintf(stderr, "[jni] JavaVM=%p functions=%p reserved0=%p (expect %p)\n",
            (void*)vm, (void*)(vm ? vm->functions : nullptr),
            vm && vm->functions ? vm->functions->reserved0 : nullptr,
            (void*)g_vm.get());
    return vm;
}

/// Ensure the C++ Java compatibility bridge is available while the engine's
/// JavaVM is provided by the experimental Rust implementation.
int roblox_jni_init_compat_bridge() {
    try {
        if (!initialize_compat_vm()) {
            return -1;
        }
        fprintf(stderr, "[jni] C++ Java compatibility bridge initialized\n");
        return 0;
    } catch (const std::exception& e) {
        fprintf(stderr, "[jni] could not initialize C++ compatibility bridge: %s\n", e.what());
        return -1;
    } catch (...) {
        fprintf(stderr, "[jni] could not initialize C++ compatibility bridge: non-standard exception\n");
        return -1;
    }
}

/// The current thread's `JNIEnv*`.
void* roblox_jni_env() {
    return g_vm ? g_vm->GetJNIEnv() : nullptr;
}

/// Does libjnivm have a real C++ hook for this exact Java method?
int roblox_jni_fallback_has_method(const char* class_name, const char* name,
                                   const char* signature, int is_static) {
    try {
        if (!g_vm || !class_name || !name || !signature) return 0;
        auto* env = roblox_runtime::process_env();
        if (!env) return 0;
        auto cls = env->GetClass(class_name);
        if (!cls) return 0;
        std::lock_guard<std::mutex> lock(cls->mtx);
        return std::any_of(cls->methods.begin(), cls->methods.end(), [&](const auto& method) {
            return method && method->name == name && method->signature == signature &&
                   method->_static == (is_static != 0) && method->nativehandle;
        }) ? 1 : 0;
    } catch (...) {
        return 0;
    }
}

int roblox_jni_fallback_has_field(const char* class_name, const char* name,
                                 const char* type, int is_static, int is_set) {
    try {
        if (!g_vm || !class_name || !name || !type) return 0;
        auto* env = roblox_runtime::process_env();
        if (!env) return 0;
        auto cls = env->GetClass(class_name);
        if (!cls) return 0;
        std::lock_guard<std::mutex> lock(cls->mtx);
        return std::any_of(cls->fields.begin(), cls->fields.end(), [&](const auto& field) {
            return field && field->name == name && field->type == type &&
                   field->_static == (is_static != 0) &&
                   (is_set ? static_cast<bool>(field->setnativehandle)
                           : static_cast<bool>(field->getnativehandle));
        }) ? 1 : 0;
    } catch (...) {
        return 0;
    }
}

int roblox_jni_fallback_get_field(const char* class_name, const char* name,
                                 const char* type, int is_static, void* receiver,
                                 jvalue* result) {
    try {
        if (!g_vm || !class_name || !name || !type || !result) return -1;
        auto* env = roblox_runtime::process_env();
        if (!env) return -1;
        JNIEnv* jni = env->GetJNIEnv();
        auto cls = env->GetClass(class_name);
        jclass jcls = static_cast<jclass>(
            jnivm::JNITypes<std::shared_ptr<jnivm::Class>>::ToJNIType(env, cls));
        jfieldID field = is_static ? jni->GetStaticFieldID(jcls, name, type)
                                   : jni->GetFieldID(jcls, name, type);
        if (!field) return -2;
        jvalue value{};
        const char kind = type[0];
        if (is_static) {
            switch (kind) {
            case 'Z': value.z = jni->GetStaticBooleanField(jcls, field); break;
            case 'B': value.b = jni->GetStaticByteField(jcls, field); break;
            case 'C': value.c = jni->GetStaticCharField(jcls, field); break;
            case 'S': value.s = jni->GetStaticShortField(jcls, field); break;
            case 'I': value.i = jni->GetStaticIntField(jcls, field); break;
            case 'J': value.j = jni->GetStaticLongField(jcls, field); break;
            case 'F': value.f = jni->GetStaticFloatField(jcls, field); break;
            case 'D': value.d = jni->GetStaticDoubleField(jcls, field); break;
            case 'L': case '[': value.l = jni->GetStaticObjectField(jcls, field); break;
            default: return -3;
            }
        } else {
            if (!receiver) return -4;
            jobject object = static_cast<jobject>(receiver);
            switch (kind) {
            case 'Z': value.z = jni->GetBooleanField(object, field); break;
            case 'B': value.b = jni->GetByteField(object, field); break;
            case 'C': value.c = jni->GetCharField(object, field); break;
            case 'S': value.s = jni->GetShortField(object, field); break;
            case 'I': value.i = jni->GetIntField(object, field); break;
            case 'J': value.j = jni->GetLongField(object, field); break;
            case 'F': value.f = jni->GetFloatField(object, field); break;
            case 'D': value.d = jni->GetDoubleField(object, field); break;
            case 'L': case '[': value.l = jni->GetObjectField(object, field); break;
            default: return -3;
            }
        }
        if (kind == 'L' || kind == '[') value.l = value.l ? jni->NewGlobalRef(value.l) : nullptr;
        *result = value;
        return 0;
    } catch (...) {
        return -5;
    }
}

int roblox_jni_fallback_set_field(const char* class_name, const char* name,
                                 const char* type, int is_static, void* receiver,
                                 jvalue value) {
    try {
        if (!g_vm || !class_name || !name || !type) return -1;
        auto* env = roblox_runtime::process_env();
        if (!env) return -1;
        JNIEnv* jni = env->GetJNIEnv();
        auto cls = env->GetClass(class_name);
        jclass jcls = static_cast<jclass>(
            jnivm::JNITypes<std::shared_ptr<jnivm::Class>>::ToJNIType(env, cls));
        jfieldID field = is_static ? jni->GetStaticFieldID(jcls, name, type)
                                   : jni->GetFieldID(jcls, name, type);
        if (!field) return -2;
        const char kind = type[0];
        if (is_static) {
            switch (kind) {
            case 'Z': jni->SetStaticBooleanField(jcls, field, value.z); break;
            case 'B': jni->SetStaticByteField(jcls, field, value.b); break;
            case 'C': jni->SetStaticCharField(jcls, field, value.c); break;
            case 'S': jni->SetStaticShortField(jcls, field, value.s); break;
            case 'I': jni->SetStaticIntField(jcls, field, value.i); break;
            case 'J': jni->SetStaticLongField(jcls, field, value.j); break;
            case 'F': jni->SetStaticFloatField(jcls, field, value.f); break;
            case 'D': jni->SetStaticDoubleField(jcls, field, value.d); break;
            case 'L': case '[': jni->SetStaticObjectField(jcls, field, value.l); break;
            default: return -3;
            }
        } else {
            if (!receiver) return -4;
            jobject object = static_cast<jobject>(receiver);
            switch (kind) {
            case 'Z': jni->SetBooleanField(object, field, value.z); break;
            case 'B': jni->SetByteField(object, field, value.b); break;
            case 'C': jni->SetCharField(object, field, value.c); break;
            case 'S': jni->SetShortField(object, field, value.s); break;
            case 'I': jni->SetIntField(object, field, value.i); break;
            case 'J': jni->SetLongField(object, field, value.j); break;
            case 'F': jni->SetFloatField(object, field, value.f); break;
            case 'D': jni->SetDoubleField(object, field, value.d); break;
            case 'L': case '[': jni->SetObjectField(object, field, value.l); break;
            default: return -3;
            }
        }
        return 0;
    } catch (...) {
        return -5;
    }
}

/// Create a persistent C++-VM object for a Rust-side opaque reference.
void* roblox_jni_fallback_new_object(const char* class_name) {
    try {
        if (!g_vm || !class_name) return nullptr;
        auto* env = roblox_runtime::process_env();
        if (!env) return nullptr;
        auto cls = env->GetClass(class_name);
        if (!cls || !cls->Instantiate) return nullptr;
        auto object = cls->Instantiate(env);
        if (!object) return nullptr;
        object->clazz = cls;
        jobject local = jnivm::JNITypes<std::shared_ptr<jnivm::Object>>::ToJNIType(env, object);
        return env->GetJNIEnv()->NewGlobalRef(local);
    } catch (...) {
        return nullptr;
    }
}

/// Mirror a Rust UTF-16 Java string into the C++ compatibility VM.
void* roblox_jni_fallback_new_string(const uint16_t* chars, int length) {
    try {
        if (!g_vm || length < 0 || (length && !chars)) return nullptr;
        auto* env = roblox_runtime::process_env();
        if (!env) return nullptr;
        JNIEnv* jni = env->GetJNIEnv();
        jstring local = jni->NewString(reinterpret_cast<const jchar*>(chars), length);
        return local ? jni->NewGlobalRef(local) : nullptr;
    } catch (...) {
        return nullptr;
    }
}

/// Invoke an already registered C++ hook. Object results are promoted to a
/// global reference so the Rust VM can retain them as opaque fallback objects.
int roblox_jni_fallback_invoke(const char* class_name, const char* name,
                               const char* signature, int is_static,
                               void* receiver, jvalue* args, jvalue* result) {
    try {
        if (!g_vm || !class_name || !name || !signature || !result) return -1;
        auto* env = roblox_runtime::process_env();
        if (!env) return -1;
        JNIEnv* jni = env->GetJNIEnv();
        auto cls = env->GetClass(class_name);
        if (!cls) return -2;
        jclass jcls = static_cast<jclass>(
            jnivm::JNITypes<std::shared_ptr<jnivm::Class>>::ToJNIType(env, cls));
        jmethodID method = is_static ? jni->GetStaticMethodID(jcls, name, signature)
                                     : jni->GetMethodID(jcls, name, signature);
        if (!method) return -2;
        const char* result_type = std::strrchr(signature, ')');
        if (!result_type || !result_type[1]) return -3;
        ++result_type;
        jvalue value{};
        if (is_static) {
            switch (*result_type) {
            case 'V': jni->CallStaticVoidMethodA(jcls, method, args); break;
            case 'Z': value.z = jni->CallStaticBooleanMethodA(jcls, method, args); break;
            case 'B': value.b = jni->CallStaticByteMethodA(jcls, method, args); break;
            case 'C': value.c = jni->CallStaticCharMethodA(jcls, method, args); break;
            case 'S': value.s = jni->CallStaticShortMethodA(jcls, method, args); break;
            case 'I': value.i = jni->CallStaticIntMethodA(jcls, method, args); break;
            case 'J': value.j = jni->CallStaticLongMethodA(jcls, method, args); break;
            case 'F': value.f = jni->CallStaticFloatMethodA(jcls, method, args); break;
            case 'D': value.d = jni->CallStaticDoubleMethodA(jcls, method, args); break;
            case 'L': case '[': value.l = jni->CallStaticObjectMethodA(jcls, method, args); break;
            default: return -3;
            }
        } else {
            if (!receiver) return -4;
            jobject object = static_cast<jobject>(receiver);
            switch (*result_type) {
            case 'V': jni->CallVoidMethodA(object, method, args); break;
            case 'Z': value.z = jni->CallBooleanMethodA(object, method, args); break;
            case 'B': value.b = jni->CallByteMethodA(object, method, args); break;
            case 'C': value.c = jni->CallCharMethodA(object, method, args); break;
            case 'S': value.s = jni->CallShortMethodA(object, method, args); break;
            case 'I': value.i = jni->CallIntMethodA(object, method, args); break;
            case 'J': value.j = jni->CallLongMethodA(object, method, args); break;
            case 'F': value.f = jni->CallFloatMethodA(object, method, args); break;
            case 'D': value.d = jni->CallDoubleMethodA(object, method, args); break;
            case 'L': case '[': value.l = jni->CallObjectMethodA(object, method, args); break;
            default: return -3;
            }
        }
        if (*result_type == 'L' || *result_type == '[') {
            value.l = value.l ? jni->NewGlobalRef(value.l) : nullptr;
        }
        *result = value;
        return 0;
    } catch (const std::exception& error) {
        fprintf(stderr, "[jnivm:fallback] C++ hook threw: %s\n", error.what());
        return -5;
    } catch (...) {
        fprintf(stderr, "[jnivm:fallback] C++ hook threw a non-standard exception\n");
        return -5;
    }
}

/// Copy a C++-VM String into a caller-owned UTF-16 buffer. Returns -1 for a
/// non-string or invalid reference; a short buffer returns the required length.
int roblox_jni_fallback_copy_string(void* string_ref, uint16_t* output, int capacity) {
    try {
        if (!g_vm || !string_ref || capacity < 0) return -1;
        auto* env = roblox_runtime::process_env();
        if (!env) return -1;
        JNIEnv* jni = env->GetJNIEnv();
        auto string = static_cast<jstring>(string_ref);
        const jsize length = jni->GetStringLength(string);
        if (length < 0) return -1;
        if (capacity < length || (length && !output)) return length;
        const jchar* chars = jni->GetStringChars(string, nullptr);
        if (!chars && length) return -1;
        if (length) std::memcpy(output, chars, static_cast<size_t>(length) * sizeof(jchar));
        if (chars) jni->ReleaseStringChars(string, chars);
        return length;
    } catch (...) {
        return -1;
    }
}

void roblox_jni_fallback_release_ref(void* object) {
    try {
        if (!g_vm || !object) return;
        auto* env = roblox_runtime::process_env();
        if (env) env->GetJNIEnv()->DeleteGlobalRef(static_cast<jobject>(object));
    } catch (...) {
    }
}

/// Write C++ stubs for every Java class and method the native code has reached
/// for so far. This is the Phase 2 backlog, observed rather than guessed.
int roblox_jni_dump_classes(const char* path) {
#ifdef JNI_DEBUG
    if (!g_vm) {
        return -1;
    }
    g_vm->GenerateClassDump(path);
    return 0;
#else
    (void)path;
    return -2;  // built without JNI_DEBUG
#endif
}

/// Call `JNI_OnLoad` with the process JavaVM, containing any C++ exception.
///
/// libjnivm reports misuse by throwing. Those exceptions originate inside
/// Roblox's call stack and would otherwise cross the Rust FFI boundary, where
/// the only outcome is `std::terminate` and a core dump — which says nothing
/// about what went wrong. Catching here turns that into a message.
///
/// Returns the JNI version on success, or one of the negative codes below.
int roblox_jni_call_onload(void* fn, char* err, size_t err_len) {
    using OnLoad = jint (*)(JavaVM*, void*);
    if (!fn || !g_vm) {
        return -1;
    }
    try {
        return reinterpret_cast<OnLoad>(fn)(&g_traced_vm, nullptr);
    } catch (const std::exception& e) {
        snprintf(err, err_len, "%s", e.what());
        return -2;
    } catch (...) {
        snprintf(err, err_len, "non-standard C++ exception");
        return -3;
    }
}

} // extern "C"

namespace roblox_runtime {
jnivm::ENV* process_env() {
    if (!g_vm || !g_real_vm) {
        return nullptr;
    }
    // NOT `g_vm->GetEnv()`. That is `jnienvs[pthread_self()]` — an
    // unordered_map::operator[], which for a thread it has never seen
    // default-constructs a null shared_ptr and returns a reference to it. Every
    // JNI hook in this build calls process_env(), and the engine calls those
    // hooks from threads it created itself, so on those threads the old version
    // handed back a null ENV and the engine stored the null where it expected a
    // JNIEnv.
    //
    // AttachCurrentThread is the entry point that does create an env for an
    // unknown thread; on a thread that already has one it finds it and changes
    // nothing. Going through it makes "which thread am I on" stop mattering.
    JNIEnv* env = nullptr;
    if (g_real_vm->AttachCurrentThread(&env, nullptr) != JNI_OK || !env) {
        return nullptr;
    }
    return jnivm::ENV::FromJNIEnv(env);
}
} // namespace roblox_runtime
