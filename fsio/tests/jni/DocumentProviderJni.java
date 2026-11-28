import java.nio.file.Path;
import java.util.concurrent.atomic.AtomicReference;

class DocumentProviderJni {
    private static native boolean verify();

    private static void fail(Exception error) throws Exception {
        throw error;
    }

    // A plain closeable result set. Loading/error/info have the same keys and
    // value types as DocumentsContract cursor extras.
    public static final class Listing implements AutoCloseable {
        private final String state;
        private boolean closed;

        Listing(String state) { this.state = state; }
        public boolean getBoolean(String key) { return key.equals("loading") && key.equals(state); }
        public String getString(String key) { return key.equals(state) ? "private-provider-message" : null; }
        public boolean hasNext() {
            if (state.equals("read failure")) throw new IllegalStateException("private-provider-message");
            return false;
        }
        public void close() { closed = true; }
        public boolean isClosed() { return closed; }
    }

    private static Object listing(String state) {
        return state.equals("null") ? null : new Listing(state);
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
