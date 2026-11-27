package dev.sunriselab.scenes.mixin;

import dev.sunriselab.scenes.SceneDriver;

import net.minecraft.client.Minecraft;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

@Mixin(Minecraft.class)
public abstract class SceneFrameMixin {
    @Inject(method = "runTick", at = @At("HEAD"))
    private void sunriseScenesFrame(boolean advanceGameTime, CallbackInfo callback) {
        SceneDriver.frame((Minecraft) (Object) this);
    }
}
