package com.stride.scan

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class SalePayloadTest {
    private var next = 0

    private fun uid(): String {
        next += 1
        return "00000000-0000-7000-8000-" + next.toString().padStart(12, '0')
    }

    private fun draft(tendered: Long = 580): SaleDraft {
        val cart = Cart()
        cart.add(productId = 3, name = "Milk", unitMinor = 250, tracked = true, newUid = ::uid)
        cart.add(productId = 3, name = "Milk", unitMinor = 250, tracked = true, newUid = ::uid)
        cart.add(productId = null, name = "Bag", unitMinor = 50, tracked = false, newUid = ::uid)

        return SalePayload.build(
            originDevice = "phone-test",
            occurredAt = "2026-11-10T08:00:00.000Z",
            userId = "staff-1",
            lines = cart.items,
            roundingMinor = 0,
            taxMinor = 0,
            tenderedMinor = tendered,
            newUid = ::uid,
        )
    }

    @Test
    fun `one payment covers the total and reports the change`() {
        val sale = draft(tendered = 600)

        assertEquals(550L, sale.totalMinor)
        assertEquals(550L, sale.payment.amountMinor)
        assertEquals(600L, sale.payment.tenderedMinor)
        assertEquals(50L, sale.payment.changeMinor)
    }

    @Test
    fun `one movement per tracked product, untracked items move no stock`() {
        val sale = draft()

        assertEquals(1, sale.movements.size)
        assertEquals(3L, sale.movements.single().productId)
        assertEquals(2, sale.movements.single().qty)
    }

    @Test(expected = IllegalArgumentException::class)
    fun `refuses cash below the amount due`() {
        draft(tendered = 549)
    }

    @Test
    fun `encodes the shape the till expects`() {
        val text = SaleJson.encode(draft())

        assertTrue(text.contains("\"uid\":\"00000000-0000-7000-8000-"))
        assertTrue(text.contains("\"origin_device\":\"phone-test\""))
        assertTrue(text.contains("\"is_offline\":1"))
        assertTrue(text.contains("\"user_id\":\"staff-1\""))
        assertTrue(text.contains("\"total_minor\":550"))
        assertTrue(text.contains("\"payments\":[{\"uid\":"))
        assertTrue(text.contains("\"method\":\"cash\""))
        assertTrue(text.contains("\"amount_minor\":550"))
        assertTrue(text.contains("\"delta\":-2"))
        assertTrue(text.contains("\"kind\":\"sale\""))
    }

    @Test
    fun `omits the user when no cashier was chosen`() {
        val cart = Cart()
        cart.add(productId = 1, name = "Soap", unitMinor = 100, tracked = false, newUid = ::uid)

        val sale = SalePayload.build(
            originDevice = "phone-test",
            occurredAt = "2026-11-10T08:00:00.000Z",
            userId = null,
            lines = cart.items,
            roundingMinor = 0,
            taxMinor = 0,
            tenderedMinor = 100,
            newUid = ::uid,
        )

        assertTrue(!SaleJson.encode(sale).contains("\"user_id\""))
    }

    @Test
    fun `escapes quotes and control characters`() {
        val cart = Cart()
        val name = "Tea \"special\"\nplease"
        cart.add(productId = null, name = name, unitMinor = 100, tracked = false, newUid = ::uid)

        val sale = SalePayload.build(
            originDevice = "phone-test",
            occurredAt = "2026-11-10T08:00:00.000Z",
            userId = null,
            lines = cart.items,
            roundingMinor = 0,
            taxMinor = 0,
            tenderedMinor = 100,
            newUid = ::uid,
        )

        val text = SaleJson.encode(sale)
        assertTrue(text.contains("\"name\":\"Tea \\\"special\\\"\\nplease\""))
    }
}
