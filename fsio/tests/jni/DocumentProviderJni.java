import java.nio.file.Path;
import java.util.concurrent.atomic.AtomicReference;

class DocumentProviderJni {
    private static native boolean verify();

    private static void fail(Exception error) throws Exception {
        throw error;
    }

    public static void main(String[] args) {
        var library = Path.of(args[0], System.mapLibraryName("document_provider_jni"));
        System.load(library.toAbsolutePath().toString());
        var uncaught = new AtomicReference<Throwable>();
        Thread.setDefaultUncaughtExceptionHandler((thread, error) -> {
            error.printStackTrace();
            uncaught.set(error);
        });
        if (!verify() || uncaught.get() != null) {
            throw new AssertionError("JNI document provider recovery failed", uncaught.get());
        }
        System.out.println("JNI document provider recovery passed");
    }
}
