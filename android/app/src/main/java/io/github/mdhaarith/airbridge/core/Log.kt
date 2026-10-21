package io.github.mdhaarith.airbridge.core

/**
 * Logging facade so the protocol core has no Android dependency (and runs in
 * plain JVM tests). The app routes it to Logcat; by default it is silent.
 */
object Log {
    fun interface Sink {
        fun log(priority: Char, tag: String, message: String, error: Throwable?)
    }

    @Volatile
    var sink: Sink = Sink { _, _, _, _ -> }

    fun d(tag: String, message: String) = sink.log('D', tag, message, null)
    fun i(tag: String, message: String) = sink.log('I', tag, message, null)
    fun w(tag: String, message: String, error: Throwable? = null) = sink.log('W', tag, message, error)
    fun e(tag: String, message: String, error: Throwable? = null) = sink.log('E', tag, message, error)
}
