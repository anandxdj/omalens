package dev.omacam.companion

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.Base64

class ControlProtocolTest {
    private val requestId = Base64.getUrlEncoder().withoutPadding().encodeToString(ByteArray(16) { 7 })

    @Test
    fun `parses exact supported start request`() {
        val response = ControlProtocol.parseResponse(
            """{"type":"start_request","request_id":"$requestId","desktop_name":"Laptop","width":1280,"height":720,"fps":30,"codec":"h264-constrained-baseline"}""",
        )

        assertEquals(ControlResponse.StartRequest(requestId, "Laptop"), response)
    }

    @Test
    fun `start request rejects unknown fields and unsupported media tuples`() {
        val valid = """{"type":"start_request","request_id":"$requestId","desktop_name":"Laptop","width":1280,"height":720,"fps":30,"codec":"h264-constrained-baseline"}"""
        assertThrows(IllegalArgumentException::class.java) {
            ControlProtocol.parseResponse(valid.dropLast(1) + ",\"extra\":true}")
        }
        for (invalid in listOf(
            valid.replace("\"width\":1280", "\"width\":1920"),
            valid.replace("\"fps\":30", "\"fps\":60"),
            valid.replace("h264-constrained-baseline", "h264-main"),
        )) {
            assertThrows(IllegalArgumentException::class.java) {
                ControlProtocol.parseResponse(invalid)
            }
        }
    }

    @Test
    fun `start request rejects malformed correlation and bounded names`() {
        val malformed = """{"type":"start_request","request_id":"short","desktop_name":"Laptop","width":1280,"height":720,"fps":30,"codec":"h264-constrained-baseline"}"""
        assertThrows(IllegalArgumentException::class.java) { ControlProtocol.parseResponse(malformed) }
        val longName = "é".repeat(33)
        assertThrows(IllegalArgumentException::class.java) {
            ControlProtocol.parseResponse(
                """{"type":"start_request","request_id":"$requestId","desktop_name":"$longName","width":1280,"height":720,"fps":30,"codec":"h264-constrained-baseline"}""",
            )
        }
    }

    @Test
    fun `capture grant preserves all binding dimensions`() {
        val response = ControlProtocol.parseResponse(
            """{"type":"capture_granted","request_id":"$requestId","peer":"${"ab".repeat(32)}","connection":"${"cd".repeat(16)}","session":"${"ef".repeat(16)}","generation":9}""",
        ) as ControlResponse.CaptureGranted

        assertEquals(requestId, response.requestId)
        assertEquals("ab".repeat(32), response.binding.peer)
        assertEquals("cd".repeat(16), response.binding.connection)
        assertEquals("ef".repeat(16), response.binding.session)
        assertEquals(9, response.binding.generation)
    }

    @Test
    fun `capture grant rejects malformed bindings and generation`() {
        val valid = """{"type":"capture_granted","request_id":"$requestId","peer":"${"ab".repeat(32)}","connection":"${"cd".repeat(16)}","session":"${"ef".repeat(16)}","generation":9}"""
        for (invalid in listOf(
            valid.replace("ab".repeat(32), "gg".repeat(32)),
            valid.replace("cd".repeat(16), "cd".repeat(15)),
            valid.replace("\"generation\":9", "\"generation\":0"),
        )) {
            assertThrows(IllegalArgumentException::class.java) {
                ControlProtocol.parseResponse(invalid)
            }
        }
    }

    @Test
    fun `pong and stop messages remain strict and bounded`() {
        val pong = ControlProtocol.parseResponse("""{"type":"pong","capture_authorized":false}""")
        assertFalse((pong as ControlResponse.Pong).captureAuthorized)
        assertEquals(
            ControlResponse.StopCapture("lease expired"),
            ControlProtocol.parseResponse("""{"type":"stop_capture","reason":"lease expired"}"""),
        )
        assertThrows(IllegalArgumentException::class.java) {
            ControlProtocol.parseResponse("""{"type":"pong","capture_authorized":false,"extra":0}""")
        }
        assertThrows(IllegalArgumentException::class.java) {
            ControlProtocol.parseResponse(
                """{"type":"stop_capture","reason":"${"x".repeat(257)}"}""",
            )
        }
    }

    @Test
    fun `oversized control input is rejected before parsing`() {
        val error = assertThrows(IllegalArgumentException::class.java) {
            ControlProtocol.parseResponse(" ".repeat(MAX_CONTROL_BYTES + 1))
        }
        assertTrue(error.message.orEmpty().contains("too large"))
    }

    @Test
    fun `commands correlate only when request id is supplied`() {
        assertEquals("start_approved", org.json.JSONObject(ControlProtocol.command("start_approved", requestId)).getString("type"))
        assertEquals(requestId, org.json.JSONObject(ControlProtocol.command("start_approved", requestId)).getString("request_id"))
        val stop = org.json.JSONObject(ControlProtocol.command("stop"))
        assertEquals(setOf("type"), stop.keys().asSequence().toSet())
    }
}
