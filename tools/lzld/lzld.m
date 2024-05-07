#import <dlfcn.h>

// -- QuartzCore.framework

extern void *kCAGravityTopLeft = 0;

// -- Metal.framework

void *(*MTLCopyAllDevices_)(void) = 0;

void loadMetalFramework() {
    void *handle = dlopen("/System/Library/Frameworks/Metal.framework/Metal", RTLD_LAZY);
    if (handle) {
        MTLCopyAllDevices_ = dlsym(handle, "MTLCopyAllDevices");
    }
}

extern void *MTLCopyAllDevices(void) {
    if (MTLCopyAllDevices_ == 0) {
        loadMetalFramework();
    }

    return MTLCopyAllDevices_();
}

// -- CoreServices.framework

void *(*FSEventStreamCreate_)(void *, void *, void *, void *, double, double, unsigned) = 0;
void *(*FSEventStreamRelease_)(void *) = 0;
void *(*FSEventStreamInvalidate_)(void *) = 0;
void *(*FSEventStreamStart_)(void *) = 0;
void *(*FSEventStreamScheduleWithRunLoop_)(void *, void *, void *) = 0;
void *(*FSEventStreamStop_)(void *) = 0;

void loadCoreServicesFramework() {
    void *handle = dlopen("/System/Library/Frameworks/CoreServices.framework/CoreServices", RTLD_LAZY);
    if (handle) {
        FSEventStreamCreate_ = dlsym(handle, "FSEventStreamCreate");
        FSEventStreamRelease_ = dlsym(handle, "FSEventStreamRelease");
        FSEventStreamInvalidate_ = dlsym(handle, "FSEventStreamInvalidate");
        FSEventStreamStart_ = dlsym(handle, "FSEventStreamStart");
        FSEventStreamScheduleWithRunLoop_ = dlsym(handle, "FSEventStreamScheduleWithRunLoop");
    }
}

extern void *FSEventStreamCreate(void *allocator, void *callback, void *context, void *paths, double sinceWhen, double latency, unsigned flags) {
    if (FSEventStreamCreate_ == 0) {
        loadCoreServicesFramework();
    }

    return FSEventStreamCreate_(allocator, callback, context, paths, sinceWhen, latency, flags);
}

extern void FSEventStreamRelease(void *streamRef) {
    if (FSEventStreamRelease_ == 0) {
        loadCoreServicesFramework();
    }

    FSEventStreamRelease_(streamRef);
}

extern void FSEventStreamInvalidate(void *streamRef) {
    if (FSEventStreamInvalidate_ == 0) {
        loadCoreServicesFramework();
    }

    FSEventStreamInvalidate_(streamRef);
}

extern void FSEventStreamStart(void *streamRef) {
    if (FSEventStreamStart_ == 0) {
        loadCoreServicesFramework();
    }

    FSEventStreamStart_(streamRef);
}

extern void FSEventStreamScheduleWithRunLoop(void *streamRef, void *runLoop, void *runLoopMode) {
    if (FSEventStreamScheduleWithRunLoop_ == 0) {
        loadCoreServicesFramework();
    }

    FSEventStreamScheduleWithRunLoop_(streamRef, runLoop, runLoopMode);
}

extern void FSEventStreamStop(void *streamRef) {
    if (FSEventStreamStop_ == 0) {
        loadCoreServicesFramework();
    }

    FSEventStreamStop_(streamRef);
}

// -- CoreFoundation.Framework

// Symbols:
// CFArrayCreateMutable
// CFRunLoopStop
// CFRunLoopIsWaiting
// CFRelease
// CFArrayAppendValue
// CFArrayGetCount
// CFRunLoopRun
// CFRunLoopGetCurrent
// CFURLCreateCopyAppendingPathComponent
// CFArrayGetValueAtIndex
// CFURLCopyFileSystemPath
// CFURLCopyFilePathURL
// CFURLCopyDeletingLastPathComponent
// CFArrayInsertValueAtIndex
// CFURLCopyLastPathComponent
// CFURLCreateFileReferenceURL
// CFURLResourceIsReachable
// CFURLCopyAbsoluteURL

