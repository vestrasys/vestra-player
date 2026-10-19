import test from 'node:test';
import assert from 'node:assert/strict';
import { bands, clamp, neutral, nodes, presets } from '../model.ts';
test('reset clears the owned chain and preset gains respect supported EQ ranges',()=>{
 assert.deepEqual(nodes(neutral()),[]);
 for(const preset of presets){assert.equal(preset.gains.length,bands.length);for(const gain of preset.gains)assert.ok(Math.abs(gain)<=12);}
 assert.throws(()=>clamp(NaN,-12,12));assert.throws(()=>clamp('4',-12,12));assert.equal(clamp(50,-12,12),12);
});
test('normalization and clarity use typed processing nodes without device volume',()=>{
 assert.deepEqual(nodes({...neutral(),normalize:true,clarity:3}),[{kind:'treble',frequency:3000,slope:0.5,gainDb:3},{kind:'dynaudnorm',enabled:true,maxGain:4,peak:0.9}]);
});
