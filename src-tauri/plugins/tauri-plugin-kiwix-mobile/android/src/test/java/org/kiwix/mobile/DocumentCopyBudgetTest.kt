package org.kiwix.mobile

import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test

class DocumentCopyBudgetTest {
    @Test
    fun entriesAreChargedOnceForTheWholeSelection() {
        val budget = DocumentCopyBudget(entryLimit = 2, byteLimit = 1024)
        budget.entry()
        budget.entry()
        val error = assertThrows(NativeDocumentError::class.java) { budget.entry() }
        assertEquals("STAGING_COPY_BUDGET_EXCEEDED", error.code)
    }

    @Test
    fun bytesAreCumulativeSoOneFileCannotResetTheBound() {
        val budget = DocumentCopyBudget(entryLimit = 4, byteLimit = 10)
        budget.charge(6)
        budget.charge(4)
        val error = assertThrows(NativeDocumentError::class.java) { budget.charge(1) }
        assertEquals("STAGING_COPY_BUDGET_EXCEEDED", error.code)
    }
}
