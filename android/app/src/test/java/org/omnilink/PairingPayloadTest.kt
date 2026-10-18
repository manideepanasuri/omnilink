package org.omnilink

import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import org.omnilink.pairing.PairingPayload

class PairingPayloadTest {

    private val validJson = """
        {
          "v": 1,
          "name": "omarchy-workstation",
          "pk": "dGVzdC1wdWJsaWMta2V5LTMyei1ieXRlcw==",
          "addrs": ["192.168.1.100:42424", "[fe80::1]:42424"],
          "token": "one_time_pairing_secret_123"
        }
    """.trimIndent()

    @Test
    fun testValidQrJsonDeserializationAndValidation() {
        val payload = PairingPayload.fromJson(validJson)
        assertNotNull(payload)
        assertTrue(payload.isValid())
        assertEquals(1, payload.version)
        assertEquals("omarchy-workstation", payload.name)
        assertEquals("dGVzdC1wdWJsaWMta2V5LTMyei1ieXRlcw==", payload.publicKeyBase64)
        assertEquals(listOf("192.168.1.100:42424", "[fe80::1]:42424"), payload.addresses)
        assertEquals("one_time_pairing_secret_123", payload.token)

        val primary = payload.primaryAddressAndPort()
        assertNotNull(primary)
        assertEquals("192.168.1.100", primary!!.first)
        assertEquals(42424, primary.second)
    }

    @Test
    fun testValidJsonViaFromJsonOrNull() {
        val payload = PairingPayload.fromJsonOrNull(validJson)
        assertNotNull(payload)
        assertTrue(payload!!.isValid())
    }

    @Test
    fun testUrlSafeBase64PublicKey() {
        val payload = PairingPayload(
            version = 1,
            name = "workstation",
            publicKeyBase64 = "dGVzdC11cmwtc2FmZS0_-_==".replace("=", ""),
            addresses = listOf("10.0.0.1:42424"),
            token = "token-xyz"
        )
        assertTrue(payload.isValid())
    }

    @Test
    fun testMalformedJsonHandling() {
        // Truncated JSON
        val truncated = """{"v": 1, "name": "workstation", "pk":"""
        assertNull(PairingPayload.fromJsonOrNull(truncated))
        try {
            PairingPayload.fromJson(truncated)
            fail("Expected exception for truncated JSON")
        } catch (_: Exception) {
            // Success
        }

        // Random non-JSON string
        val plainText = "https://omnilink.org/download"
        assertNull(PairingPayload.fromJsonOrNull(plainText))
        try {
            PairingPayload.fromJson(plainText)
            fail("Expected exception for plain text")
        } catch (_: Exception) {
            // Success
        }

        // Empty string
        assertNull(PairingPayload.fromJsonOrNull(""))
    }

    @Test
    fun testMissingRequiredFields() {
        // Missing "v"
        val missingVersion = """{"name":"pc","pk":"dGVzdA==","addrs":["1.1.1.1:42424"],"token":"t"}"""
        assertNull(PairingPayload.fromJsonOrNull(missingVersion))

        // Missing "name"
        val missingName = """{"v":1,"pk":"dGVzdA==","addrs":["1.1.1.1:42424"],"token":"t"}"""
        assertNull(PairingPayload.fromJsonOrNull(missingName))

        // Missing "pk"
        val missingPk = """{"v":1,"name":"pc","addrs":["1.1.1.1:42424"],"token":"t"}"""
        assertNull(PairingPayload.fromJsonOrNull(missingPk))

        // Missing "addrs"
        val missingAddrs = """{"v":1,"name":"pc","pk":"dGVzdA==","token":"t"}"""
        assertNull(PairingPayload.fromJsonOrNull(missingAddrs))

        // Missing "token"
        val missingToken = """{"v":1,"name":"pc","pk":"dGVzdA==","addrs":["1.1.1.1:42424"]}"""
        assertNull(PairingPayload.fromJsonOrNull(missingToken))
    }

    @Test
    fun testInvalidVersion() {
        val payloadV2 = PairingPayload(
            version = 2,
            name = "pc",
            publicKeyBase64 = "dGVzdA==",
            addresses = listOf("1.1.1.1:42424"),
            token = "token"
        )
        assertFalse(payloadV2.isValid())

        val payloadV0 = payloadV2.copy(version = 0)
        assertFalse(payloadV0.isValid())
    }

    @Test
    fun testBlankOrEmptyName() {
        val emptyName = PairingPayload(
            version = 1,
            name = "",
            publicKeyBase64 = "dGVzdA==",
            addresses = listOf("1.1.1.1:42424"),
            token = "token"
        )
        assertFalse(emptyName.isValid())

        val blankName = emptyName.copy(name = "   ")
        assertFalse(blankName.isValid())
    }

    @Test
    fun testInvalidBase64PublicKey() {
        val blankPk = PairingPayload(
            version = 1,
            name = "pc",
            publicKeyBase64 = "   ",
            addresses = listOf("1.1.1.1:42424"),
            token = "token"
        )
        assertFalse(blankPk.isValid())

        val invalidCharsPk = blankPk.copy(publicKeyBase64 = "!!!not-valid-base64???")
        assertFalse(invalidCharsPk.isValid())
    }

    @Test
    fun testEmptyOrBlankAddresses() {
        val emptyAddrs = PairingPayload(
            version = 1,
            name = "pc",
            publicKeyBase64 = "dGVzdA==",
            addresses = emptyList(),
            token = "token"
        )
        assertFalse(emptyAddrs.isValid())

        val onlyBlankAddrs = emptyAddrs.copy(addresses = listOf("", "   "))
        assertFalse(onlyBlankAddrs.isValid())
        assertNull(onlyBlankAddrs.primaryAddressAndPort())
    }

    @Test
    fun testBlankOrEmptyToken() {
        val emptyToken = PairingPayload(
            version = 1,
            name = "pc",
            publicKeyBase64 = "dGVzdA==",
            addresses = listOf("1.1.1.1:42424"),
            token = ""
        )
        assertFalse(emptyToken.isValid())

        val blankToken = emptyToken.copy(token = "   ")
        assertFalse(blankToken.isValid())
    }

    @Test
    fun testAddressParsingVariations() {
        // Standard IPv4 with port
        val (h1, p1) = PairingPayload.parseAddress("192.168.1.100:42424")
        assertEquals("192.168.1.100", h1)
        assertEquals(42424, p1)

        // IPv6 with port
        val (h2, p2) = PairingPayload.parseAddress("[fe80::1]:42424")
        assertEquals("fe80::1", h2)
        assertEquals(42424, p2)

        // Hostname with custom port
        val (h3, p3) = PairingPayload.parseAddress("my-desktop.local:8080")
        assertEquals("my-desktop.local", h3)
        assertEquals(8080, p3)

        // IP without port (default 42424)
        val (h4, p4) = PairingPayload.parseAddress("192.168.1.50")
        assertEquals("192.168.1.50", h4)
        assertEquals(42424, p4)

        // Invalid port out of bounds (falls back to 42424)
        val (h5, p5) = PairingPayload.parseAddress("192.168.1.50:99999")
        assertEquals("192.168.1.50", h5)
        assertEquals(42424, p5)
    }

    @Test
    fun testSerializationRoundTrip() {
        val original = PairingPayload(
            version = 1,
            name = "roundtrip-pc",
            publicKeyBase64 = "dGVzdC1rZXk=",
            addresses = listOf("10.0.0.5:42424"),
            token = "secret123"
        )

        val jsonString = Json.encodeToString(original)
        val parsed = PairingPayload.fromJson(jsonString)
        assertEquals(original, parsed)
        assertTrue(parsed.isValid())
    }

    @Test
    fun testEd25519TinkKeyGeneration() {
        val keyPair = com.google.crypto.tink.subtle.Ed25519Sign.KeyPair.newKeyPair()
        assertEquals(32, keyPair.publicKey.size)
        assertEquals(32, keyPair.privateKey.size)

        val pubBase64 = java.util.Base64.getEncoder().encodeToString(keyPair.publicKey)
        assertTrue(PairingPayload.isValidBase64(pubBase64))
    }
}
