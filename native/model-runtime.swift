import Foundation
import CoreML
import Darwin
@main struct ModelRuntime {
    static func main() async {
        do {
            guard CommandLine.arguments.count==2 else {throw NSError(domain:"ParakeetModel",code:1,userInfo:[NSLocalizedDescriptionKey:"Expected one JSON request file"])}
            let parent=getppid();if parent==1{exit(1)};let watchdog=DispatchSource.makeTimerSource(queue:DispatchQueue.global());watchdog.schedule(deadline:.now()+1,repeating:1);watchdog.setEventHandler{if getppid() != parent{exit(1)}};watchdog.resume();defer{watchdog.cancel()}
            let data=try Data(contentsOf:URL(fileURLWithPath:CommandLine.arguments[1]));let o=try JSONSerialization.jsonObject(with:data) as! [String:Any]
            if (o["task"] as? String)=="asr" {let reply=try await runAsr(o);let output=try JSONSerialization.data(withJSONObject:reply,options:[.sortedKeys]);FileHandle.standardOutput.write(output);FileHandle.standardOutput.write(Data([10]));return}
            let directory=URL(fileURLWithPath:o["directory"] as! String);let audio=URL(fileURLWithPath:o["audio_f32"] as! String)
            let started=Date();var config=(o["preset"] as? String)=="offline" ? Nemotron3Config.offline : Nemotron3Config.fast128
            config.modelFileName="monolithic/v2/"+config.modelFileName
            let models=try await Nemotron3Models.load(config:config,directory:directory,computeUnits:.cpuAndGPU);let loaded=Date().timeIntervalSince(started)
            let pcm=try Data(contentsOf:audio,options:.mappedIfSafe);guard pcm.count%4==0 else {throw NSError(domain:"ParakeetModel",code:2,userInfo:[NSLocalizedDescriptionKey:"Truncated float32 PCM"])}
            let diarizer=Nemotron3Diarizer(config:config,models:models);var probabilities:[Float]=[];var frames=0
            func append(_ chunks:[Nemotron3ChunkResult]) {for chunk in chunks {probabilities.append(contentsOf:chunk.probabilities);frames+=chunk.frameCount}}
            try pcm.withUnsafeBytes { raw in let samples=raw.bindMemory(to:Float.self);for offset in stride(from:0,to:samples.count,by:16000){try autoreleasepool {diarizer.appendAudio(Array(samples[offset..<min(samples.count,offset+16000)]));append(try diarizer.processBufferedAudio())};if offset % 80000 == 0 {let progress:[String:Any] = ["event":"inference_progress","completed_ms":min(frames*10,samples.count/16),"total_ms":samples.count/16];if let data=try? JSONSerialization.data(withJSONObject:progress){FileHandle.standardError.write(data);FileHandle.standardError.write(Data([10]))}}};append(try diarizer.finishStream())}
            let intervals=Nemotron3Diarizer.segments(probabilities:probabilities,frameCount:frames,numSpeakers:8,threshold:0.5,frameSeconds:0.01,minDurationSeconds:0).map {s->[String:Any] in ["start_ms":Int((s.startSeconds*1000).rounded()),"end_ms":Int((s.endSeconds*1000).rounded()),"speaker":"SPEAKER_\(s.speakerIndex)"]}
            diarizer.reset()
            var usage=rusage();let measured=getrusage(RUSAGE_SELF,&usage)==0
            var result:[String:Any]=["intervals":intervals,"model_load_seconds":loaded,"wall_seconds":Date().timeIntervalSince(started),"audio_seconds":Double(pcm.count/4)/16000,"threshold":0.5,"compute_units":"cpu_and_gpu","preset":o["preset"] as? String ?? "fast128","implementation":"FluidAudio subset 1b0ed50d686873128a4129b3510ce3914c849bab"]
            if measured {result["helper_peak_rss_bytes"]=Int64(usage.ru_maxrss)}
            let output=try JSONSerialization.data(withJSONObject:result,options:[.sortedKeys]);FileHandle.standardOutput.write(output);FileHandle.standardOutput.write(Data([10]))
        }catch{let value:[String:Any]=["error":error.localizedDescription];if let output=try? JSONSerialization.data(withJSONObject:value){FileHandle.standardOutput.write(output)};exit(1)}
    }
}
