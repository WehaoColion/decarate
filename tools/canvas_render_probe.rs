// v0.0.1 - Host probe for the exact prior draw loop and current cached edge renderer.
pub const PROBE: &str = r####"package com.ofairyo.gridtimer.ui
import androidx.compose.ui.geometry.Offset
import kotlinx.serialization.json.*
import kotlin.math.*
import java.lang.management.ManagementFactory

private class ProbePaint { var color = 0; var strokeWidth = 0f }
private class ProbeCanvas(val capture: Boolean = false) {
    var calls = 0; var used = 0; var checksum = 0.0
    val coordinates = if(capture) FloatArray(48000) else FloatArray(0)
    fun reset() { calls=0; used=0; checksum=0.0 }
    private fun consume(a: Float, b: Float, c: Float, d: Float) {
        checksum += (a+b+c+d).toDouble()
        if(capture) { coordinates[used]=a; coordinates[used+1]=b; coordinates[used+2]=c; coordinates[used+3]=d }
        used+=4
    }
    fun drawLine(a: Float,b: Float,c: Float,d: Float,paint: ProbePaint) { calls++; consume(a,b,c,d) }
    fun drawLines(lines: FloatArray, offset: Int, count: Int) {
        calls++; var i=offset
        while(i<offset+count) { consume(lines[i],lines[i+1],lines[i+2],lines[i+3]); i+=4 }
    }
}
private interface FrameRenderer { fun frame(dragId: String?, dragDelta: Offset, native: ProbeCanvas) }
private class PriorRenderer(private val canvas: CanvasBoard): FrameRenderer {
    private val nodeIndex=canvas.nodes.associateBy { it.id }
    private val paint=ProbePaint()
    override fun frame(dragId: String?,dragDelta: Offset,native: ProbeCanvas) {
        val viewport=CanvasViewport(0f,0f,1f)
        val left=-20000f; val top=-20000f; val right=20000f; val bottom=20000f; val lineColor=0
        __PRIOR_EDGE_BODY__
    }
}
private class CachedRenderer(canvas: CanvasBoard): FrameRenderer {
    val renderer=CanvasEdgeRenderer(canvas.nodes,canvas.edges)
    override fun frame(dragId: String?,dragDelta: Offset,native: ProbeCanvas) {
        val count=renderer.updateVisible(-20000f,-20000f,20000f,20000f,dragId,dragDelta)
        if(count>0) native.drawLines(renderer.visibleLines,0,count)
    }
}
private data class Measurement(val micros: Double,val bytes: Double,val calls: Int,val checksum: Double)
fun main() {
    val nodes=List(1000) { i -> CanvasNode("n$i","note","","content",(i%50)*320f,(i/50)*240f,256f,168f,"blue") }
    val edges=(0 until 1000).flatMap { i -> (1..4).map { j -> CanvasEdge("e${i}_$j","n$i","n${(i+j)%1000}") } }
    val board=CanvasBoard("board","Benchmark",0f,0f,1f,nodes,edges)
    val bean=ManagementFactory.getThreadMXBean() as com.sun.management.ThreadMXBean
    check(bean.isThreadAllocatedMemorySupported); bean.isThreadAllocatedMemoryEnabled=true
    val thread=Thread.currentThread().id
    fun allocation()=bean.getThreadAllocatedBytes(thread)
    val beforeAllocated=allocation(); val beforeStart=System.nanoTime(); val prior=PriorRenderer(board)
    val beforeInitMicros=(System.nanoTime()-beforeStart)/1000.0; val beforeInitBytes=allocation()-beforeAllocated
    val afterAllocated=allocation(); val afterStart=System.nanoTime(); val cached=CachedRenderer(board)
    val afterInitMicros=(System.nanoTime()-afterStart)/1000.0; val afterInitBytes=allocation()-afterAllocated
    var equivalentFrames=0
    for(i in 0..31) {
        val old=ProbeCanvas(true); val next=ProbeCanvas(true)
        val id=if(i==0) null else "n${i*29}"
        val delta=Offset(i*17.25f,-i*13.5f)
        prior.frame(id,delta,old); cached.frame(id,delta,next)
        check(old.used==next.used)
        repeat(old.used) { j -> check(abs(old.coordinates[j]-next.coordinates[j])<=max(0.002f,abs(old.coordinates[j])*0.000001f)) { "Geometry changed at frame $i coordinate $j" } }
        check(next.calls==1); equivalentFrames++
    }
    val sink=ProbeCanvas()
    fun measure(renderer: FrameRenderer,drag: Boolean,frames: Int): Measurement {
        sink.reset(); val allocated=allocation(); val started=System.nanoTime()
        repeat(frames) { i -> renderer.frame(if(drag) "n499" else null,if(drag) Offset((i%50)*3f,(i%40)*2f) else Offset.Zero,sink) }
        val elapsed=System.nanoTime()-started; val bytes=allocation()-allocated
        check(sink.checksum.isFinite() && sink.checksum>0)
        return Measurement(elapsed/1000.0/frames,bytes.toDouble()/frames,sink.calls/frames,sink.checksum)
    }
    val scenarios=buildJsonArray {
        for(drag in listOf(false,true)) {
            repeat(3) { measure(prior,drag,500); measure(cached,drag,500) }
            val before=mutableListOf<Measurement>(); val after=mutableListOf<Measurement>()
            repeat(25) { round ->
                if(round%2==0) { before+=measure(prior,drag,100); after+=measure(cached,drag,100) }
                else { after+=measure(cached,drag,100); before+=measure(prior,drag,100) }
            }
            fun summary(samples: List<Measurement>)=buildJsonObject {
                val times=samples.map { it.micros }.sorted()
                put("medianMicrosPerFrame",times[times.size/2]); put("p95MicrosPerFrame",times[23])
                put("meanAllocatedBytesPerFrame",samples.map { it.bytes }.average()); put("drawCallsPerFrame",samples.first().calls)
                put("samples",buildJsonArray { samples.forEach { add(it.micros) } })
            }
            add(buildJsonObject { put("operation",if(drag) "drag" else "pan"); put("before",summary(before)); put("after",summary(after)) })
        }
    }
    println(buildJsonObject {
        put("passed",true); put("hostOnly",true); put("nodes",nodes.size); put("edges",edges.size); put("equivalentGeometryFrames",equivalentFrames)
        put("rounds",25); put("framesPerSample",100); put("scenarios",scenarios)
        put("beforeInitMicros",beforeInitMicros); put("beforeInitAllocatedBytes",beforeInitBytes)
        put("afterInitMicros",afterInitMicros); put("afterInitAllocatedBytes",afterInitBytes)
        put("cachedFloatBufferBytes",(edges.size*28+16)*4)
        put("measurementScope","Desktop JVM, real geometry and culling code with a coordinate-consuming draw sink; excludes Android rasterization, GPU, display and persistence. Allocation traffic is not retained memory or RSS.")
    })
}
"####;
