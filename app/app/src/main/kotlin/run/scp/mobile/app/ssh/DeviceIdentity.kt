package run.scp.mobile.app.ssh

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import java.io.File
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import run.scp.mobile.scpmobile.Scpmobile

/** The device's SSH identity. The public half is what an admin adds to the server's authorized_keys. */
interface DeviceIdentity {
    fun privateKeyPem(): ByteArray
    fun publicAuthorizedKey(): String
}

/**
 * Generates an Ed25519 key on first use. The private key is stored only encrypted
 * (AES-256-GCM) under a non-exportable Android Keystore key; the public line is stored plain.
 */
class KeystoreDeviceIdentity(context: Context) : DeviceIdentity {
    private val dir = context.filesDir
    private val privFile = File(dir, "device_key.enc")
    private val pubFile = File(dir, "device_key.pub")
    private val lock = Any()

    override fun privateKeyPem(): ByteArray = synchronized(lock) {
        ensure()
        decrypt(privFile.readBytes())
    }

    override fun publicAuthorizedKey(): String = synchronized(lock) {
        ensure()
        pubFile.readText()
    }

    private fun ensure() {
        if (privFile.exists() && pubFile.exists()) return
        val pair = Scpmobile.generateEd25519()
        // Write the private key first; a missing .pub means "regenerate", never a mismatched pair.
        pubFile.delete()
        privFile.writeBytes(encrypt(pair.privateOpenSSH()))
        pubFile.writeText(String(pair.publicAuthorizedKey(), Charsets.UTF_8))
    }

    private fun wrapKey(): SecretKey {
        val ks = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
        (ks.getKey(ALIAS, null) as? SecretKey)?.let { return it }
        val gen = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, ANDROID_KEYSTORE)
        gen.init(
            KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build(),
        )
        return gen.generateKey()
    }

    private fun encrypt(plain: ByteArray): ByteArray {
        val c = Cipher.getInstance(TRANSFORM).apply { init(Cipher.ENCRYPT_MODE, wrapKey()) }
        return c.iv + c.doFinal(plain) // Keystore generates a fresh 12-byte IV
    }

    private fun decrypt(blob: ByteArray): ByteArray {
        val c = Cipher.getInstance(TRANSFORM).apply {
            init(Cipher.DECRYPT_MODE, wrapKey(), GCMParameterSpec(128, blob, 0, IV_BYTES))
        }
        return c.doFinal(blob, IV_BYTES, blob.size - IV_BYTES)
    }

    private companion object {
        const val ANDROID_KEYSTORE = "AndroidKeyStore"
        const val ALIAS = "scp-mobile-device-key-wrap"
        const val TRANSFORM = "AES/GCM/NoPadding"
        const val IV_BYTES = 12
    }
}
