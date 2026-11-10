package com.stride.scan

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class MoneyTextTest {
    @Test
    fun `parses whole shillings`() {
        assertEquals(1_200L, MoneyText.parse("1200", 0))
        assertEquals(1_200L, MoneyText.parse("1,200", 0))
        assertEquals(1_200L, MoneyText.parse(" 1200 ", 0))
    }

    @Test
    fun `parses decimals when the currency has them`() {
        assertEquals(1_250L, MoneyText.parse("12.50", 2))
        assertEquals(1_200L, MoneyText.parse("12", 2))
        assertEquals(25L, MoneyText.parse(".25", 2))
    }

    @Test
    fun `refuses a decimal place the currency cannot hold`() {
        assertNull(MoneyText.parse("12.5", 0))
        assertNull(MoneyText.parse("12.505", 2))
    }

    @Test
    fun `refuses negatives and junk`() {
        assertNull(MoneyText.parse("-1", 0))
        assertNull(MoneyText.parse("", 0))
        assertNull(MoneyText.parse("1.2.3", 2))
        assertNull(MoneyText.parse("one", 0))
    }

    @Test
    fun `formats with grouping and a sign`() {
        assertEquals("1,200", MoneyText.format(1_200, 0))
        assertEquals("12.50", MoneyText.format(1_250, 2))
        assertEquals("-12.05", MoneyText.format(-1_205, 2))
    }
}
