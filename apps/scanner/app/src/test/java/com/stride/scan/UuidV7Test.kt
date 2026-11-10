package com.stride.scan

import java.util.UUID
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class UuidV7Test {
    @Test
    fun `has the RFC shape and the version nibble the till checks`() {
        val id = UuidV7.generate(1_700_000_000_000)

        assertEquals(36, id.length)
        assertEquals('7', id[14])
        assertTrue(id[19] in "89ab")
    }

    @Test
    fun `the till can parse it as a UUID`() {
        val parsed = UUID.fromString(UuidV7.generate(1_700_000_000_000))
        assertEquals(7, parsed.version())
        assertEquals(2, parsed.variant())
    }

    @Test
    fun `later timestamps sort later`() {
        val tail = ByteArray(10) { it.toByte() }

        val earlier = UuidV7.generate(1_700_000_000_000, tail)
        val later = UuidV7.generate(1_700_000_000_001, tail)

        assertTrue(later > earlier)
    }
}
