package com.remotedesktop.mobile

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * Ulanish kalitlarini xavfsiz saqlash (desktop'dagi `secrets.rs` bilan bir xil mantiq). Kalit faqat foydalanuvchi
 * shu ulanish uchun "Saqlash" ni belgilagan bo'lsa saqlanadi; har bir ulanishning (P2P manzili yoki ID) o'z yozuvi bor.
 *
 * Shifrlash: Android Keystore'dagi AES-256/GCM kaliti (qurilmadan chiqarib bo'lmaydi, imkon bo'lsa apparat ichida).
 * SharedPreferences'ga faqat `IV + shifrlangan matn` yoziladi; ochiq kalit diskka hech qachon tushmaydi.
 */
class SavedKeys(ctx: Context) {
    private val prefs = ctx.getSharedPreferences("saved_keys", Context.MODE_PRIVATE)

    fun get(conn: String): String? {
        val blob = prefs.getString(conn, null) ?: return null
        return try {
            val raw = Base64.decode(blob, Base64.NO_WRAP)
            if (raw.size <= IV_LEN) return null
            val c = Cipher.getInstance(TRANSFORM)
            c.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, raw, 0, IV_LEN))
            c.updateAAD(conn.toByteArray()) // yozuv boshqa ulanishga ko'chirilsa ochilmaydi
            String(c.doFinal(raw, IV_LEN, raw.size - IV_LEN), Charsets.UTF_8).ifEmpty { null }
        } catch (e: Exception) {
            // Keystore kaliti yo'qolgan (masalan, qurilma qulfi o'zgargan) yoki yozuv buzilgan: yaroqsiz yozuv o'chiriladi
            delete(conn)
            null
        }
    }

    /** Kalitni saqlaydi (oldingisi almashtiriladi). Bo'sh kalit saqlanmaydi: yozuv o'chiriladi. */
    fun set(conn: String, password: String): Boolean {
        if (password.isEmpty()) {
            delete(conn)
            return true
        }
        return try {
            val c = Cipher.getInstance(TRANSFORM)
            c.init(Cipher.ENCRYPT_MODE, key())
            c.updateAAD(conn.toByteArray())
            val enc = c.doFinal(password.toByteArray(Charsets.UTF_8))
            val iv = c.iv
            prefs.edit().putString(conn, Base64.encodeToString(iv + enc, Base64.NO_WRAP)).apply()
            true
        } catch (e: Exception) {
            false
        }
    }

    fun delete(conn: String) {
        if (prefs.contains(conn)) prefs.edit().remove(conn).apply()
    }

    private fun key(): SecretKey {
        val ks = KeyStore.getInstance(KEYSTORE).apply { load(null) }
        (ks.getKey(ALIAS, null) as? SecretKey)?.let { return it }
        val gen = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, KEYSTORE)
        gen.init(
            KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build(),
        )
        return gen.generateKey()
    }

    companion object {
        private const val KEYSTORE = "AndroidKeyStore"
        private const val ALIAS = "ulan_saved_keys_v1"
        private const val TRANSFORM = "AES/GCM/NoPadding"
        private const val IV_LEN = 12

        /** P2P ulanish yozuvi (`manzil[:port]`). */
        fun p2pKey(addr: String) = "p2p:${addr.trim().lowercase()}"

        /** Server orqali ulanish yozuvi (9 xonali ID). */
        fun idKey(id: String) = "id:${id.trim()}"
    }
}
