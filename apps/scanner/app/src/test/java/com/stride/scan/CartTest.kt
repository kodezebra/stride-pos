package com.stride.scan

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class CartTest {
    private fun uid(): String = UuidV7.generate()

    @Test
    fun `merges the same catalog product`() {
        val cart = Cart()
        cart.add(productId = 7, name = "Milk", unitMinor = 1_200, tracked = true)
        cart.add(productId = 7, name = "Milk", unitMinor = 1_200, tracked = true)

        assertEquals(1, cart.items.size)
        assertEquals(2, cart.items.first().qty)
        assertEquals(2_400L, cart.totalMinor())
    }

    @Test
    fun `merges a manual item by name and price`() {
        val cart = Cart()
        cart.add(productId = null, name = "Bread", unitMinor = 500, tracked = false, newUid = ::uid)
        cart.add(productId = null, name = " bread ", unitMinor = 500, tracked = false, newUid = ::uid)
        cart.add(productId = null, name = "Bread", unitMinor = 600, tracked = false, newUid = ::uid)

        assertEquals(2, cart.items.size)
        assertEquals(2, cart.items.first().qty)
    }

    @Test
    fun `a change to zero removes the line`() {
        val cart = Cart()
        cart.add(productId = 1, name = "Soap", unitMinor = 300, tracked = true)
        val line = cart.items.single()

        cart.change(line.uid, -1)
        assertTrue(cart.isEmpty)
    }

    @Test
    fun `rounds cash half away from the smallest coin`() {
        val cart = Cart()
        cart.add(productId = 1, name = "Sugar", unitMinor = 340, tracked = true)

        assertEquals(-40L, cart.roundingMinor(100))
        assertEquals(300L, cart.dueMinor(100))
    }

    @Test
    fun `rounds up when the remainder is at least half a coin`() {
        val cart = Cart()
        cart.add(productId = 1, name = "Salt", unitMinor = 350, tracked = true)

        assertEquals(50L, cart.roundingMinor(100))
        assertEquals(400L, cart.dueMinor(100))
    }

    @Test
    fun `tax is the part already inside the total`() {
        val cart = Cart()
        cart.add(productId = 1, name = "Rice", unitMinor = 1_800, tracked = true)

        assertEquals(300L, cart.taxMinor(20))
    }

    @Test
    fun `no tax and no rounding leave the total alone`() {
        val cart = Cart()
        cart.add(productId = 1, name = "Tea", unitMinor = 2_000, tracked = true)

        assertEquals(0L, cart.taxMinor(0))
        assertEquals(0L, cart.roundingMinor(1))
        assertEquals(2_000L, cart.dueMinor(1))
    }
}
